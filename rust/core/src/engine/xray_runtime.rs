//! Thin Rust wrapper over the `xray_bridge` Go cgo library.
//!
//! On Android we dynamically link against `libxray_bridge.so` (built from
//! `native/xray_bridge` — see that directory's README). The three C ABI
//! entrypoints — `xray_start`, `xray_stop`, `FreeCString` — give us full
//! control over the lifecycle of an in-process xray-core instance.
//!
//! TUN ↔ SOCKS bridging is handled by [`hev_runtime::HevTunnel`], which
//! lives in a separate dynamically-linked C library
//! (`libhev-socks5-tunnel.so`). Keeping the two responsibilities split
//! lets us replace either side independently.
//!
//! On every other target (used only for `cargo check` / unit tests on the
//! CI host) we expose the same public surface but every call returns
//! [`Error::XrayUnavailable`]. This keeps the rest of the engine
//! buildable and unit-testable on Linux/macOS without dragging the entire
//! Go toolchain into host CI.
//!
//! ## Threading & ownership
//!
//! The Go side guards xray mutation with a `sync.Mutex`, so the FFI is
//! safe to call from any thread but is effectively a singleton: at most
//! one xray instance at a time. The Rust handle below is an RAII guard
//! that calls `xray_stop` on `Drop`, so the caller cannot forget to
//! clean up.
//!
//! Strings returned by the bridge MUST be freed with `FreeCString`; we do
//! that immediately after copying into a Rust `String`.

use crate::error::{Error, Result};

#[cfg(target_os = "android")]
mod sys {
    use std::os::raw::c_char;

    extern "C" {
        pub fn xray_start(config_json: *const c_char) -> *mut c_char;
        pub fn xray_stop() -> *mut c_char;
        pub fn FreeCString(s: *mut c_char);
    }
}

/// RAII guard for the singleton xray-core instance. Drops back to "no
/// xray running" on `Drop`.
pub struct XrayInstance {
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
