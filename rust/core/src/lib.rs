//! `ff_vpn_core` — cross-platform VPN core for the fictional-octo-spork client.
//!
//! Architecture (strict):
//!   TUN → Leaf #1 (TUN inbound → SOCKS5 outbound)
//!       → Arti (Tor with Pluggable Transport over SOCKS5)
//!       → Leaf #2 (SOCKS5 inbound → VLESS+Reality+gRPC outbound)
//!       → Internet
//!
//! All VPN-critical logic is implemented in Rust. C/C++ is only used at the
//! FFI boundary (JNI on Android, C ABI on iOS Network Extension, Dart FFI on
//! desktop).

#![allow(clippy::missing_safety_doc)]

pub mod config;
pub mod dns;
pub mod engine;
pub mod error;
pub mod ipc;
pub mod pt;
pub mod runtime;
pub mod util;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub mod ffi;

pub use error::{Error, Result};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tracing::info;

/// Lazily-constructed multi-threaded Tokio runtime owned by the library.
fn rt() -> &'static Runtime {
    static RT: OnceCell<Runtime> = OnceCell::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("ff-vpn-core")
            .build()
            .expect("failed to build tokio runtime")
    })
}

/// Process-wide handle to the current VPN engine, if running.
fn current_engine() -> &'static Mutex<Option<Arc<engine::Engine>>> {
    static SLOT: OnceCell<Mutex<Option<Arc<engine::Engine>>>> = OnceCell::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Initialise logging exactly once. Safe to call from any platform shim.
pub fn init_logging() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static INIT: AtomicBool = AtomicBool::new(false);
    if INIT.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,leaf=info,arti=info")),
        )
        .with_target(false)
        .try_init();
    info!("ff_vpn_core logging initialised");
}

/// Start the VPN engine with the given parsed configuration and platform context.
///
/// The TUN file descriptor (`tun_fd`) must already be opened by the platform
/// (Android `VpnService.Builder.establish()` or iOS `NEPacketTunnelProvider`)
/// because mobile OSes do not allow user-space processes to create TUNs.
///
/// **fd ownership contract:** once this function is called, the Rust core owns
/// `ctx.tun_fd` and is responsible for closing it on failure (via
/// [`util::FdGuard`]) — the platform layer must NOT close it on a non-zero
/// return code. This prevents the double-close race that occurs when both
/// sides try to clean up a descriptor whose number POSIX may have reassigned.
pub fn start(cfg: config::AppConfig, ctx: engine::PlatformContext) -> Result<engine::EngineHandle> {
    init_logging();
    // Single fd-ownership point: armed before any fallible step (including
    // the `AlreadyRunning` check) and disarmed only after `Engine::start`
    // has successfully transferred the fd to Leaf #2's TUN inbound.
    let fd_guard = util::FdGuard::new(ctx.tun_fd);
    let mut slot = current_engine().lock();
    if slot.is_some() {
        return Err(Error::AlreadyRunning);
    }
    let engine = rt().block_on(async move { engine::Engine::start(cfg, ctx).await })?;
    let handle = engine.handle();
    *slot = Some(engine);
    fd_guard.disarm();
    Ok(handle)
}

/// Stop the VPN engine immediately. Returns once all background tasks have been
/// cancelled and resources released.
pub fn stop() -> Result<()> {
    let engine = {
        let mut slot = current_engine().lock();
        slot.take()
    };
    if let Some(engine) = engine {
        rt().block_on(async move { engine.stop().await });
    }
    Ok(())
}

/// Snapshot the current engine status (for the UI / dev tab).
pub fn status() -> engine::Status {
    current_engine()
        .lock()
        .as_ref()
        .map(|e| e.status_snapshot())
        .unwrap_or_default()
}
