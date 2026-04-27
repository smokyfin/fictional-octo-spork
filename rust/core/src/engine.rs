//! VPN engine — the orchestrator that wires Leaf #1 → Arti → Leaf #2 together
//! and owns the platform-supplied TUN file descriptor.

use parking_lot::Mutex;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::config::AppConfig;
use crate::error::Result;
use crate::pt::{self, PluggableTransport};
use crate::runtime::{Cancel, SocksEndpoint};

pub mod arti_runtime;
pub mod leaf_config;

/// Information the platform must give the engine when starting. The TUN file
/// descriptor itself is created by the OS — Android's `VpnService.Builder` /
/// iOS `NEPacketTunnelProvider` — and we keep the Rust side platform-agnostic.
#[derive(Debug, Clone)]
pub struct PlatformContext {
    /// Owned TUN file descriptor opened by the platform layer.
    pub tun_fd: i32,

    /// IP assigned to the TUN interface inside the VPN session — also the
    /// address our embedded DNS server listens on.
    pub tun_addr: IpAddr,

    /// MTU for the TUN interface.
    pub tun_mtu: u16,

    /// Application-private writable directory.
    pub private_dir: PathBuf,

    /// Provider selection for the Pluggable Transport layer.
    pub pt_provider: pt::Provider,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Status {
    pub running: bool,
    pub pt_provider: Option<String>,
    pub pt_socks: Option<String>,
    pub arti_socks: Option<String>,
    pub tun_addr: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EngineHandle {
    cancel: Cancel,
}

impl EngineHandle {
    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

pub struct Engine {
    cancel: Cancel,
    status: Arc<Mutex<Status>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    pt: Mutex<Option<Box<dyn PluggableTransport>>>,
}

/// Closes a raw fd when dropped unless `disarm()` is called. Used to make
/// sure the platform-supplied TUN fd is reclaimed if `Engine::start` fails
/// partway through bringup — the platform layer (Kotlin / Swift) has already
/// detached its `ParcelFileDescriptor`/`NWTCPConnection`, so without this
/// guard a failed start would leak one TUN descriptor per attempt.
#[cfg(unix)]
struct FdGuard(Option<i32>);

#[cfg(unix)]
impl FdGuard {
    fn new(fd: i32) -> Self {
        Self(Some(fd))
    }
    fn disarm(mut self) {
        let _ = self.0.take();
    }
}

#[cfg(unix)]
impl Drop for FdGuard {
    fn drop(&mut self) {
        if let Some(fd) = self.0.take() {
            // SAFETY: fd was supplied by the platform layer and ownership was
            // handed to us; closing it here mirrors POSIX `close(2)`.
            unsafe {
                libc::close(fd);
            }
        }
    }
}

#[cfg(not(unix))]
struct FdGuard;

#[cfg(not(unix))]
impl FdGuard {
    fn new(_fd: i32) -> Self {
        Self
    }
    fn disarm(self) {}
}

impl Engine {
    pub async fn start(cfg: AppConfig, ctx: PlatformContext) -> Result<Arc<Self>> {
        // Reclaim the FD if any of the bring-up steps below fail. Disarmed
        // on the success path so the engine keeps owning it for the whole
        // session.
        let fd_guard = FdGuard::new(ctx.tun_fd);

        let cancel = Cancel::new();
        let status = Arc::new(Mutex::new(Status {
            running: true,
            tun_addr: Some(ctx.tun_addr.to_string()),
            ..Status::default()
        }));

        // 1. Pluggable Transport (Leaf #1) — Arti will use this SOCKS as its
        //    bridge transport. Random port + auth.
        let pt = pt::spawn(ctx.pt_provider, cancel.clone()).await?;
        info!(
            provider = pt.name(),
            socks = %pt.socks().addr,
            "PT layer up"
        );
        status.lock().pt_provider = Some(pt.name().to_string());
        status.lock().pt_socks = Some(pt.socks().addr.to_string());

        // 2. Arti (Tor) — listens on a random local SOCKS port; uses the PT
        //    SOCKS as its bridge transport.
        let arti_socks = SocksEndpoint::new_random()?;
        let arti_task = arti_runtime::spawn_arti(
            cfg.clone(),
            pt.socks().clone(),
            arti_socks.clone(),
            ctx.private_dir.clone(),
            cancel.clone(),
        )
        .await?;
        info!(socks = %arti_socks.addr, "Arti up");
        status.lock().arti_socks = Some(arti_socks.addr.to_string());

        // 3. Leaf #2 — TUN inbound + VLESS+Reality outbound chain (and SOCKS
        //    pre-stage that hands traffic off to Arti).
        let leaf_main_cfg = leaf_config::main_engine_config(&cfg, &ctx, &arti_socks)?;
        let leaf_task =
            leaf_config::run_leaf_with_config("leaf-main", leaf_main_cfg, cancel.clone())?;
        info!("Leaf #2 (TUN→VLESS) up");

        // 4. Embedded DNS resolver bound to the TUN address — proxies queries
        //    through the VLESS path to the configured DoH server.
        let dns_task = crate::dns::spawn_dns_proxy(
            cfg.clone(),
            ctx.tun_addr,
            arti_socks.clone(),
            cancel.clone(),
        )
        .await?;

        // 5. IPC server — UI ↔ engine bridge. Lives in the private dir.
        let ipc_task =
            crate::ipc::spawn_ipc_server(ctx.private_dir.clone(), status.clone(), cancel.clone())
                .await?;

        let engine = Arc::new(Engine {
            cancel,
            status,
            tasks: Mutex::new(vec![arti_task, leaf_task, dns_task, ipc_task]),
            pt: Mutex::new(Some(pt)),
        });

        // From here on the engine owns the TUN fd via Leaf #2's tun inbound;
        // disarm the guard so we don't double-close it on success.
        fd_guard.disarm();

        Ok(engine)
    }

    pub fn handle(&self) -> EngineHandle {
        EngineHandle {
            cancel: self.cancel.clone(),
        }
    }

    pub fn status_snapshot(&self) -> Status {
        self.status.lock().clone()
    }

    pub async fn stop(self: Arc<Self>) {
        info!("VPN engine stopping");
        self.cancel.cancel();

        // Tear down PT first so Arti gets clean EOFs upstream. Take the
        // pointer out of the mutex inside a small scope so the (non-Send)
        // parking_lot guard is dropped before we hit `.await`.
        let pt = { self.pt.lock().take() };
        if let Some(pt) = pt {
            pt.shutdown().await;
        }

        let tasks = std::mem::take(&mut *self.tasks.lock());
        for task in tasks {
            task.abort();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
        }
        self.status.lock().running = false;
        info!("VPN engine stopped");
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[allow(dead_code)]
fn _silence_error<T>(r: Result<T>) {
    if let Err(e) = r {
        error!(?e, "engine internal error");
    }
}
