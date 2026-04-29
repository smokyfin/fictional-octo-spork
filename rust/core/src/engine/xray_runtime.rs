//! Thin Rust wrapper over the `xray_bridge` Go cgo library.
//!
//! On Android we dynamically link against `libxray_bridge.so` (built from
//! `native/xray_bridge` — see that directory's README). The five C ABI
//! entrypoints — `xray_start`, `xray_stop`, `tun2socks_start`,
//! `tun2socks_stop`, `FreeCString` — give us full control over the
//! lifecycle of an in-process xray-core instance and a `tun2socks/v2`
//! engine that bridges the Android TUN file descriptor into it.
//!
//! On every other target (used only for `cargo check` / unit tests on the
//! CI host) we expose the same public surface but every call returns
//! [`Error::XrayUnavailable`]. This keeps the rest of the engine
//! buildable and unit-testable on Linux/macOS without dragging the entire
//! Go toolchain into host CI.
//!
//! ## Threading & ownership
//!
//! The Go side guards both `xray_*` and `tun2socks_*` mutation with a
//! single `sync.Mutex`, so the FFI is safe to call from any thread but is
//! effectively a singleton: at most one xray instance and one tun2socks
//! engine at a time. The Rust handles below are RAII guards that call the
//! corresponding `*_stop` on `Drop`, so the caller cannot forget to clean
//! up.
//!
//! Strings returned by the bridge MUST be freed with `FreeCString`; we do
//! that immediately after copying into a Rust `String`.

use crate::error::{Error, Result};

#[cfg(target_os = "android")]
mod sys {
    use std::os::raw::{c_char, c_int};

    extern "C" {
        pub fn xray_start(config_json: *const c_char) -> *mut c_char;
        pub fn xray_stop() -> *mut c_char;
        pub fn tun2socks_start(
            fd: c_int,
            mtu: c_int,
            proxy_url: *const c_char,
            udp_timeout_ms: c_int,
        ) -> *mut c_char;
        pub fn tun2socks_stop() -> *mut c_char;
        pub fn FreeCString(s: *mut c_char);
    }
}

/// RAII guard for the singleton xray-core instance. Drops back to "no
/// xray running" on `Drop`.
pub struct XrayInstance {
    _private: (),
}

/// RAII guard for the tun2socks engine. Drops back to "tun closed" on
/// `Drop`. The TUN fd it was given is closed by the engine itself.
pub struct Tun2SocksHandle {
    _private: (),
}

#[cfg(target_os = "android")]
fn take_c_string(ptr: *mut std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: every Go bridge entrypoint allocates with `C.CString` and
    // returns a NUL-terminated string we own; we copy it out, then free
    // through the bridge's `FreeCString` (which calls libc `free` on Go's
    // side). Calling `CStr::from_ptr` is safe because the pointer is
    // non-null, NUL-terminated, and lives until we free it below.
    let s = unsafe {
        std::ffi::CStr::from_ptr(ptr)
            .to_string_lossy()
            .into_owned()
    };
    unsafe { sys::FreeCString(ptr) };
    s
}

impl XrayInstance {
    /// Bring up an xray-core instance with the supplied JSON configuration
    /// (the raw `xray-core` config schema, NOT our `AppConfig`'s
    /// persistence schema). At most one instance may be alive at a time.
    pub fn start(config_json: &str) -> Result<Self> {
        #[cfg(target_os = "android")]
        {
            let cstr = std::ffi::CString::new(config_json)
                .map_err(|_| Error::InvalidConfig("xray config contains NUL byte".into()))?;
            let err = take_c_string(unsafe { sys::xray_start(cstr.as_ptr()) });
            if err.is_empty() {
                Ok(Self { _private: () })
            } else {
                Err(Error::Engine(format!("xray_start: {err}")))
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = config_json;
            Err(Error::XrayUnavailable)
        }
    }
}

impl Drop for XrayInstance {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        {
            let err = take_c_string(unsafe { sys::xray_stop() });
            if !err.is_empty() {
                tracing::warn!(error = %err, "xray_stop returned an error");
            }
        }
    }
}

impl Tun2SocksHandle {
    /// Wire the supplied TUN file descriptor through `tun2socks/v2` to a
    /// SOCKS5 endpoint (typically xray's local socks-inbound). The fd is
    /// duplicated by the engine; the caller must NOT close the original
    /// after a successful return.
    pub fn start(
        tun_fd: i32,
        mtu: u16,
        socks_url: &str,
        udp_timeout: std::time::Duration,
    ) -> Result<Self> {
        #[cfg(target_os = "android")]
        {
            let cstr = std::ffi::CString::new(socks_url)
                .map_err(|_| Error::InvalidConfig("socks URL contains NUL byte".into()))?;
            let timeout_ms: i32 = udp_timeout
                .as_millis()
                .try_into()
                .unwrap_or(i32::MAX);
            let err = take_c_string(unsafe {
                sys::tun2socks_start(
                    tun_fd,
                    i32::from(mtu),
                    cstr.as_ptr(),
                    timeout_ms,
                )
            });
            if err.is_empty() {
                Ok(Self { _private: () })
            } else {
                Err(Error::Engine(format!("tun2socks_start: {err}")))
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (tun_fd, mtu, socks_url, udp_timeout);
            Err(Error::XrayUnavailable)
        }
    }
}

impl Drop for Tun2SocksHandle {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        {
            let err = take_c_string(unsafe { sys::tun2socks_stop() });
            if !err.is_empty() {
                tracing::warn!(error = %err, "tun2socks_stop returned an error");
            }
        }
    }
}
