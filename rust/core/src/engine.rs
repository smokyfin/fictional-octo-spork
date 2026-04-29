//! VPN engine — the orchestrator.
//!
//! Pipeline (default, "direct" mode):
//!
//! ```text
//!   TUN fd ──┐                    ┌──> VLESS+Reality+(grpc|xhttp) outbound
//!            └─> tun2socks/v2 ──> SOCKS5 ──> xray-core ──> Internet
//! ```
//!
//! Pipeline ("via Tor", UI toggle):
//!
//! ```text
//!   TUN fd ──┐
//!            └─> tun2socks/v2 ──> SOCKS5 ──> xray-core ──┐
//!                                                       │
//!                                  arti-client SOCKS <──┘
//!                                  (Tor) ──> VLESS exit
//! ```
//!
//! `tun2socks/v2` and `xray-core` are both Go libraries we link in via the
//! `xray_bridge` cgo shim (`native/xray_bridge`). They are exposed to Rust
//! through [`xray_runtime::XrayInstance`] and
//! [`xray_runtime::Tun2SocksHandle`], both RAII-managed.
//!
//! Arti runs in-process on the Tokio runtime owned by this crate; the
//! engine wires its SOCKS5 listener as an upstream proxy of xray's VLESS
//! outbound (`streamSettings.sockopt.dialerProxy = "arti-socks"`).

use parking_lot::Mutex;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::runtime::{Cancel, SocksEndpoint};

pub mod arti_runtime;
pub mod xray_config;
pub mod xray_runtime;

// Kept compiling for now — the leaf path is no longer reachable but the
// module still type-checks against the pinned `leaf` crate. We will drop
// it (and the workspace dependency) in a follow-up once the xray pipeline
// is verified end-to-end on device.
#[allow(dead_code)]
pub mod leaf_config;

/// Information the platform must give the engine when starting. The TUN file
/// descriptor itself is created by the OS — Android's `VpnService.Builder` /
/// iOS `NEPacketTunnelProvider` — and we keep the Rust side platform-agnostic.
#[derive(Debug, Clone)]
pub struct PlatformContext {
    /// Owned TUN file descriptor opened by the platform layer.
    pub tun_fd: i32,

    /// IP assigned to the TUN interface inside the VPN session.
    pub tun_addr: IpAddr,

    /// MTU for the TUN interface.
    pub tun_mtu: u16,

    /// Application-private writable directory.
    pub private_dir: PathBuf,

    /// Provider selection for the (legacy) Pluggable Transport layer.
    /// Retained for ABI compatibility with the platform shims; the
    /// xray-based pipeline does not consult it.
    pub pt_provider: crate::pt::Provider,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Status {
    pub running: bool,
    pub pt_provider: Option<String>,
    pub pt_socks: Option<String>,
    pub arti_socks: Option<String>,
    pub tun_addr: Option<String>,
    /// Local SOCKS5 endpoint xray exposes to tun2socks (debug surface).
    pub xray_socks: Option<String>,
    /// `"direct"` or `"via-tor"` depending on the user's UI toggle.
    pub route_mode: Option<String>,
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
    /// Tun2socks engine — must be dropped *before* xray so it stops
    /// pumping packets into a dead socks listener.
    tun: Mutex<Option<xray_runtime::Tun2SocksHandle>>,
    /// xray-core instance.
    xray: Mutex<Option<xray_runtime::XrayInstance>>,
}

impl Engine {
    /// `ctx.tun_fd` is owned by the caller (`crate::start`) until this
    /// function returns `Ok` — see the `FdGuard` in `lib.rs::start`. We do
    /// **not** close the fd here on failure; doing so would race with the
    /// guard and could double-close (closing an unrelated fd that POSIX
    /// reassigned in the meantime).
    pub async fn start(cfg: AppConfig, ctx: PlatformContext) -> Result<Arc<Self>> {
        let cancel = Cancel::new();
        let status = Arc::new(Mutex::new(Status {
            running: true,
            tun_addr: Some(ctx.tun_addr.to_string()),
            ..Status::default()
        }));

        // 1. Decide route mode based on the UI toggle.
        let via_tor = cfg.user.route_through_tor;
        status.lock().route_mode = Some(
            if via_tor { "via-tor" } else { "direct" }.to_string(),
        );

        // 2. Optionally bring up Arti first — xray's config below will
        //    point its VLESS outbound at this SOCKS endpoint.
        let mut tasks: Vec<JoinHandle<()>> = Vec::new();
        let arti_socks_opt = if via_tor {
            let arti_socks = SocksEndpoint::new_random()?;
            // Re-use the existing Arti spawner; the obsolete `pt_socks`
            // argument is kept around purely to satisfy its current
            // signature. Bridges are disabled inside `build_arti_config`.
            let pt_socks = SocksEndpoint::new_random()?;
            let arti_task = arti_runtime::spawn_arti(
                cfg.clone(),
                pt_socks,
                arti_socks.clone(),
                ctx.private_dir.clone(),
                cancel.clone(),
            )
            .await?;
            tasks.push(arti_task);
            status.lock().arti_socks = Some(arti_socks.addr.to_string());
            info!(addr = %arti_socks.addr, "Arti SOCKS up");
            Some(arti_socks.addr)
        } else {
            None
        };

        // 3. Random SOCKS5 endpoint that xray will expose to tun2socks.
        let xray_socks = SocksEndpoint::new_random()?.addr;
        status.lock().xray_socks = Some(xray_socks.to_string());

        // 4. Emit the xray JSON config and start the xray instance.
        let xray_cfg_value = match arti_socks_opt {
            Some(arti) => xray_config::build_via_tor(&cfg, xray_socks, arti),
            None => xray_config::build_direct(&cfg, xray_socks),
        };
        let xray_cfg_json = serde_json::to_string(&xray_cfg_value).map_err(Error::from)?;
        info!(
            socks = %xray_socks,
            via_tor,
            network = %cfg.outbound.network,
            "starting xray-core",
        );
        let xray = xray_runtime::XrayInstance::start(&xray_cfg_json)?;

        // 5. Wire the TUN fd into tun2socks pointed at xray's SOCKS in.
        //    From here on, the Go side owns the fd; the Rust caller's
        //    `FdGuard` will be disarmed on a successful return.
        let proxy_url = format!("socks5://{xray_socks}");
        let tun = match xray_runtime::Tun2SocksHandle::start(
            ctx.tun_fd,
            ctx.tun_mtu,
            &proxy_url,
            Duration::from_secs(60),
        ) {
            Ok(h) => h,
            Err(e) => {
                // Make sure xray is torn down before we propagate, so we
                // don't leak the singleton instance.
                drop(xray);
                cancel.cancel();
                return Err(e);
            }
        };
        info!(proxy = %proxy_url, "tun2socks engaged");

        // 6. IPC server — UI ↔ engine bridge.
        let ipc_task = crate::ipc::spawn_ipc_server(
            ctx.private_dir.clone(),
            status.clone(),
            cancel.clone(),
        )
        .await?;
        tasks.push(ipc_task);

        let engine = Arc::new(Engine {
            cancel,
            status,
            tasks: Mutex::new(tasks),
            tun: Mutex::new(Some(tun)),
            xray: Mutex::new(Some(xray)),
        });
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

        // Drop tun2socks first so xray stops seeing new client conns.
        {
            let _ = self.tun.lock().take();
        }
        // Then xray.
        {
            let _ = self.xray.lock().take();
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
