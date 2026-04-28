//! Generates Leaf JSON configurations for the two Leaf instances in the
//! pipeline, and runs them on a dedicated background task.
//!
//! Leaf accepts its own JSON schema (see `leaf/src/config/common.rs`).
//! The schema we emit here is what `leaf::config::json::from_string` will
//! consume.
//!
//! NOTE on gRPC: leaf v0.14.2 ships TLS / Reality / WebSocket / QUIC /
//! AMux transports out of the box, but does **not** expose a stand-alone
//! `grpc` outbound. The `grpc_service_name` we keep in `AppConfig` is still
//! parsed and persisted so that, when leaf gains gRPC (or when we swap
//! the PT provider for Xray-core), we already have it. Today we wire
//! VLESS-over-Reality (Reality is the transport, VLESS is the payload)
//! through leaf's standard `chain` actor.

use crate::config::AppConfig;
use crate::engine::PlatformContext;
use crate::error::{Error, Result};
use crate::runtime::{Cancel, SocksEndpoint};
use rand::Rng;
use serde_json::json;
use tokio::task::JoinHandle;
use tracing::{error, info};

use leaf::{Config as LeafConfig, RuntimeOption, StartOptions};

/// Runtime ids must be unique across concurrent leaf instances inside the
/// process. We pick high random numbers to avoid colliding with anything a
/// host application might be using.
fn fresh_runtime_id() -> u16 {
    rand::rng().random_range(1024..u16::MAX)
}

/// Leaf #1 — used as the Pluggable-Transport SOCKS5 proxy that Arti dials its
/// bridge through. Only a SOCKS5 inbound + a `direct` outbound are needed;
/// the bridge handshake itself rides over Arti's transport plugin.
pub fn pt_socks_inbound_config(socks: &SocksEndpoint) -> Result<String> {
    let cfg = json!({
        "log": { "level": "info" },
        "inbounds": [{
            "tag": "socks-pt",
            "protocol": "socks",
            "address": socks.addr.ip().to_string(),
            "port": socks.addr.port(),
            "settings": {
                "username": socks.username,
                "password": socks.password,
            }
        }],
        "outbounds": [{
            "tag": "out",
            "protocol": "direct"
        }]
    });
    Ok(serde_json::to_string(&cfg)?)
}

/// Leaf #2 — the main engine that owns the TUN inbound and the
/// VLESS+Reality outbound chain. Its TUN inbound forwards traffic to
/// `proxy → [socks-arti, vless-reality-chain]`.
///
/// `vless-reality-chain` is itself a leaf `chain` outbound consisting of
/// `[reality, vless]` so that the Reality TLS handshake wraps the VLESS
/// payload.
pub fn main_engine_config(
    cfg: &AppConfig,
    ctx: &PlatformContext,
    arti_socks: &SocksEndpoint,
) -> Result<String> {
    let reality = &cfg.outbound.reality;

    let leaf_cfg = json!({
        "log": { "level": "info" },
        "inbounds": [
            {
                "tag": "tun-in",
                "protocol": "tun",
                "settings": {
                    "fd": ctx.tun_fd,
                    "auto": false,
                    "mtu": ctx.tun_mtu as i32,
                    "name": "tun-ff",
                    "address": ctx.tun_addr.to_string(),
                    "fakeDnsExclude": []
                }
            }
        ],
        "outbounds": [
            // Top-level outbound the router targets — a chain that first goes
            // through Arti's SOCKS, then through the Reality+VLESS sub-chain.
            {
                "tag": "proxy",
                "protocol": "chain",
                "settings": {
                    "actors": ["socks-arti", "vless-reality-chain"]
                }
            },
            {
                "tag": "socks-arti",
                "protocol": "socks",
                "settings": {
                    "address": arti_socks.addr.ip().to_string(),
                    "port": arti_socks.addr.port(),
                    "username": arti_socks.username,
                    "password": arti_socks.password
                }
            },
            // Reality + VLESS chain. The Reality actor performs the TLS-like
            // handshake that mimics `serverName` and pins the public key;
            // the VLESS actor speaks the proxy protocol on top.
            {
                "tag": "vless-reality-chain",
                "protocol": "chain",
                "settings": { "actors": ["reality-tls", "vless-out"] }
            },
            {
                "tag": "reality-tls",
                "protocol": "reality",
                "settings": {
                    "serverName": reality.server_name,
                    "publicKey":  reality.public_key,
                    "shortId":    reality.short_id
                }
            },
            {
                "tag": "vless-out",
                "protocol": "vless",
                "settings": {
                    "address": cfg.outbound.address,
                    "port":    cfg.outbound.port,
                    "uuid":    cfg.outbound.user_id
                }
            },
            { "tag": "direct", "protocol": "direct" },
            { "tag": "drop",   "protocol": "drop"   }
        ],
        // Send everything to the proxy chain. Per-app filtering is handled at
        // the OS level (Android `addAllowedApplication`/`addDisallowedApplication`);
        // on iOS the Network Extension defines include/exclude routes.
        "router": {
            "rules": [
                { "ip": ["0.0.0.0/0", "::/0"], "target": "proxy" }
            ]
        }
    });
    Ok(serde_json::to_string(&leaf_cfg)?)
}

/// Spawn a Leaf runtime on a blocking thread. Cancellation triggers
/// `leaf::shutdown(rt_id)` which causes `leaf::start` to return.
pub fn run_leaf_with_config(
    name: &'static str,
    config: String,
    cancel: Cancel,
) -> Result<JoinHandle<()>> {
    let rt_id = fresh_runtime_id();
    info!(name, rt_id, "spawning leaf runtime");

    // Watcher: trigger leaf shutdown on cancel.
    let cancel_watch = cancel.clone();
    let shutdown_watch = tokio::spawn(async move {
        cancel_watch.cancelled().await;
        leaf::shutdown(rt_id);
    });

    let blocking = tokio::task::spawn_blocking(move || {
        let opts = StartOptions {
            config: LeafConfig::Str(config),
            // Worker auto-tuned, default stack size.
            runtime_opt: RuntimeOption::MultiThreadAuto(2 * 1024 * 1024),
        };
        if let Err(e) = leaf::start(rt_id, opts) {
            error!(?e, name, "leaf runtime exited with error");
        }
        cancel.cancel();
        shutdown_watch.abort();
    });

    Ok(tokio::spawn(async move {
        if let Err(join) = blocking.await {
            error!(?join, name, "leaf join error");
        }
    }))
}

#[allow(dead_code)]
fn _err(s: impl Into<String>) -> Error {
    Error::Leaf(s.into())
}
