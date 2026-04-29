//! VPN engine — the orchestrator.
//!
//! Pipeline (default, "via Arti"):
//!
//! ```text
//!   TUN fd ──┐
//!            └─> hev-socks5-tunnel ──> SOCKS5 (Arti)
//!                                       │
//!                                       └─> Tor circuit ──> SOCKS5 (xray-PT)
//!                                                                │
//!                                                                └─> VLESS+Reality+(grpc|xhttp)
//!                                                                          │
//!                                                                          └─> server (`127.0.0.1:9001` after VLESS)
//! ```
//!
//! Pipeline (`skip_arti=true`, bypass mode):
//!
//! ```text
//!   TUN fd ──┐
//!            └─> hev-socks5-tunnel ──> SOCKS5 (xray) ──> VLESS+Reality ──> server
//! ```
//!
//! Components:
//! - **hev-socks5-tunnel** (C library): TUN ↔ SOCKS5 bridge with mapdns.
//! - **arti-client** (Rust crate, ≥ 0.41): Tor client with bridge support;
//!   reaches the server bridge through xray-core acting as an unmanaged
//!   Pluggable Transport.
//! - **xray-core** (Go, via cgo `libxray_bridge.so`): VLESS+Reality+gRPC/xhttp
//!   outbound layer.
//!
//! Startup ordering matters — we must NOT bring TUN online (= start hev)
//! until the upstream chain is reachable. Concretely:
//!
//!   1. start xray-core with the SOCKS-in address pre-allocated
//!   2. (if `!skip_arti`) start Arti, configured to use xray's SOCKS-in
//!      as an unmanaged PT for the configured bridge; wait for bootstrap
//!   3. start hev-socks5-tunnel pointing at either Arti's SOCKS (default)
//!      or directly at xray's SOCKS (`skip_arti=true`)
//!   4. flip status to `Connected` only after step 3 succeeds

use parking_lot::Mutex;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::runtime::{Cancel, SocksEndpoint};

pub mod arti_runtime;
pub mod hev_runtime;
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
    /// Local SOCKS5 endpoint xray exposes (debug surface).
    pub xray_socks: Option<String>,
    /// `"via-arti"` or `"skip-arti"` depending on `effective_skip_arti`.
    pub route_mode: Option<String>,
    /// `"connecting"` while Arti bootstraps / xray starts; `"connected"`
    /// only after hev attaches to the TUN fd; `"stopping"` during
    /// teardown.
    pub phase: Option<String>,
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
    /// hev-socks5-tunnel — must be dropped *before* xray/Arti so it
    /// stops pumping packets into a dead socks listener.
    hev: Mutex<Option<hev_runtime::HevTunnel>>,
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
            phase: Some("connecting".into()),
            ..Status::default()
        }));

        // Decide the route mode based on config + UI override.
        let skip_arti = cfg.user.effective_skip_arti(cfg.skip_arti);
        status.lock().route_mode = Some(
            if skip_arti { "skip-arti" } else { "via-arti" }.to_string(),
        );

        let mut tasks: Vec<JoinHandle<()>> = Vec::new();

        // 1. Allocate the SOCKS5 endpoint xray will listen on. We do this
        //    *before* starting xray because Arti's PT config needs to
        //    point at it, and we want to fail fast on port-exhaustion.
        let xray_socks = SocksEndpoint::new_random()?.addr;
        status.lock().xray_socks = Some(xray_socks.to_string());

        // 2. Emit xray JSON and start the xray instance. xray's role
        //    is identical regardless of skip_arti: be a transparent
        //    SOCKS5→VLESS+Reality proxy.
        let xray_cfg_value = xray_config::build_direct(&cfg, xray_socks);
        let xray_cfg_json = serde_json::to_string(&xray_cfg_value).map_err(Error::from)?;
        info!(
            socks = %xray_socks,
            skip_arti,
            network = %cfg.outbound.network,
            "starting xray-core (PT layer)",
        );
        let xray = xray_runtime::XrayInstance::start(&xray_cfg_json)?;

        // 3. Optionally bring up Arti. When Arti is in the chain it's
        //    the SOCKS endpoint hev hands packets to; otherwise hev
        //    talks directly to xray.
        let hev_target_socks = if skip_arti {
            xray_socks
        } else {
            let arti_socks = SocksEndpoint::new_random()?;
            // The PT proxy address Arti dials for bridges is xray's
            // SOCKS-in. We re-use the SocksEndpoint type to ferry the
            // address across the spawn boundary.
            let pt_socks = SocksEndpoint {
                addr: xray_socks,
                username: String::new(),
                password: String::new(),
            };
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
            arti_socks.addr
        };

        // 4. Start hev-socks5-tunnel — this is the moment TUN goes
        //    "online" because hev attaches to the fd and starts pumping
        //    packets. From here on the Go/C side owns the fd; Rust's
        //    FdGuard is disarmed on a successful return.
        let tun_v4 = match ctx.tun_addr {
            IpAddr::V4(v4) => v4,
            IpAddr::V6(_) => {
                return Err(Error::Engine(
                    "TUN must be assigned an IPv4 address (got IPv6)".into(),
                ));
            }
        };
        let yaml = hev_runtime::build_yaml(
            tun_v4,
            ctx.tun_mtu,
            hev_target_socks,
            None,
            None,
            // mapdns: hev intercepts UDP/53 to the TUN IP and translates
            // queries into DNS-over-SOCKS to the upstream proxy, which
            // (because of xray's DNS hijack rule) routes them through
            // DoH. This replaces the dedicated DNS proxy we used in the
            // leaf-based pipeline.
            Some(tun_v4),
        );
        info!(
            target = %hev_target_socks,
            tun_fd = ctx.tun_fd,
            "starting hev-socks5-tunnel (TUN online)"
        );
        let hev = match hev_runtime::HevTunnel::start(&yaml, ctx.tun_fd) {
            Ok(h) => h,
            Err(e) => {
                drop(xray);
                cancel.cancel();
                return Err(e);
            }
        };

        // 5. IPC server — UI ↔ engine bridge.
        let ipc_task = crate::ipc::spawn_ipc_server(
            ctx.private_dir.clone(),
            status.clone(),
            cancel.clone(),
        )
        .await?;
        tasks.push(ipc_task);

        status.lock().phase = Some("connected".into());

        let engine = Arc::new(Engine {
            cancel,
            status,
            tasks: Mutex::new(tasks),
            hev: Mutex::new(Some(hev)),
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
        self.status.lock().phase = Some("stopping".into());
        self.cancel.cancel();

        // Drop hev first so xray/arti stop seeing new client conns.
        {
            let _ = self.hev.lock().take();
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
        self.status.lock().phase = Some("disconnected".into());
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
