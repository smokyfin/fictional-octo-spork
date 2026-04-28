//! Mobile platform FFI bindings exposed by `cdylib`/`staticlib`.
//!
//! These are intentionally narrow C ABI wrappers — every interesting decision
//! lives in Rust. The Android JNI crate (`ff_vpn_jni`) reuses these.

use crate::config::AppConfig;
use crate::engine::PlatformContext;
use crate::pt::Provider;
use crate::util::FdGuard;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

/// Last error message, in thread-local storage. NUL-terminated UTF-8.
thread_local! {
    static LAST_ERR: std::cell::RefCell<Option<CString>> =
        const { std::cell::RefCell::new(None) };
}

fn set_last_err(msg: impl Into<String>) {
    LAST_ERR.with(|cell| {
        *cell.borrow_mut() = CString::new(msg.into()).ok();
    });
}

/// Returns a borrowed pointer to the last error string set on this thread,
/// or NULL if there is none. The pointer is valid until the next FFI call on
/// the same thread.
#[no_mangle]
pub extern "C" fn ff_vpn_last_error() -> *const c_char {
    LAST_ERR.with(|cell| match &*cell.borrow() {
        Some(s) => s.as_ptr(),
        None => std::ptr::null(),
    })
}

/// Initialise platform logging. Idempotent.
#[no_mangle]
pub extern "C" fn ff_vpn_init() {
    crate::init_logging();
}

/// Start the VPN engine. All `*const c_char` parameters must be NUL-terminated
/// UTF-8 strings; ownership stays with the caller.
///
/// `config_json` — the *full* parsed `AppConfig` JSON (the platform layer
///                  fetched + parsed it, or restored it from disk).
/// `private_dir` — application-private directory.
/// `tun_fd`     — file descriptor for the TUN interface (already established).
/// `tun_addr`   — IP assigned to the TUN; the embedded DNS server binds here.
/// `tun_mtu`    — MTU for the interface.
///
/// Returns 0 on success, non-zero on failure (and sets `ff_vpn_last_error`).
#[no_mangle]
pub unsafe extern "C" fn ff_vpn_start(
    config_json: *const c_char,
    private_dir: *const c_char,
    tun_fd: i32,
    tun_addr: *const c_char,
    tun_mtu: u16,
) -> i32 {
    // Mirror the JNI shim's fd-ownership pattern: arm a guard on the raw
    // fd we received from the platform, and only disarm it once we've
    // handed off to `crate::start` (which arms its own guard). On iOS the
    // platform passes -1 because the Network Extension uses `packetFlow`
    // instead of a raw fd; `FdGuard` no-ops on negative values.
    let fd_guard = FdGuard::new(tun_fd);
    let res = (|| -> crate::Result<()> {
        let cfg = cstr_to_str(config_json)?;
        let dir = cstr_to_str(private_dir)?;
        let addr = cstr_to_str(tun_addr)?;
        let parsed: AppConfig = serde_json::from_str(cfg)?;
        let ctx = PlatformContext {
            tun_fd,
            tun_addr: addr.parse().map_err(|e: std::net::AddrParseError| {
                crate::Error::InvalidConfig(e.to_string())
            })?,
            tun_mtu,
            private_dir: PathBuf::from(dir),
            pt_provider: Provider::Leaf,
        };
        fd_guard.disarm();
        crate::start(parsed, ctx).map(|_| ())
    })();
    match res {
        Ok(()) => 0,
        Err(e) => {
            set_last_err(e.to_string());
            1
        }
    }
}

/// Stop the engine. Returns 0 on success.
#[no_mangle]
pub extern "C" fn ff_vpn_stop() -> i32 {
    match crate::stop() {
        Ok(()) => 0,
        Err(e) => {
            set_last_err(e.to_string());
            1
        }
    }
}

/// Returns a freshly-allocated NUL-terminated JSON status string. The caller
/// must free it with `ff_vpn_free_string`.
#[no_mangle]
pub extern "C" fn ff_vpn_status_json() -> *mut c_char {
    let s = serde_json::to_string(&crate::status()).unwrap_or_else(|_| "{}".to_string());
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string returned by this library.
#[no_mangle]
pub unsafe extern "C" fn ff_vpn_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

/// Returns the per-session IPC auth token (newly allocated). Caller frees
/// with `ff_vpn_free_string`.
#[no_mangle]
pub extern "C" fn ff_vpn_ipc_auth_token() -> *mut c_char {
    let t = crate::ipc::current_auth_token();
    match CString::new(t) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

unsafe fn cstr_to_str<'a>(p: *const c_char) -> crate::Result<&'a str> {
    if p.is_null() {
        return Err(crate::Error::Other("null FFI pointer".into()));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|e| crate::Error::Other(format!("invalid utf-8: {e}")))
}
