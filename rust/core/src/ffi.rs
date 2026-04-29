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

// Last error message. Stored in a global Mutex (not a thread_local) because
// the JNI side runs on the Android binder thread pool — successive calls
// (`start` then `lastError`) are not guaranteed to land on the same thread.
static LAST_ERR: parking_lot::Mutex<Option<CString>> = parking_lot::Mutex::new(None);

pub fn set_last_err(msg: impl Into<String>) {
    *LAST_ERR.lock() = CString::new(msg.into()).ok();
}

pub fn take_last_error_string() -> Option<String> {
    LAST_ERR
        .lock()
        .take()
        .and_then(|c| c.into_string().ok())
}

/// Returns a freshly-allocated NUL-terminated copy of the last error string,
/// or NULL if none. Caller must free with `ff_vpn_free_string`. Safe to call
/// from any thread.
#[no_mangle]
pub extern "C" fn ff_vpn_last_error() -> *mut c_char {
    let mut slot = LAST_ERR.lock();
    match slot.take() {
        Some(s) => s.into_raw(),
        None => std::ptr::null_mut(),
    }
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
