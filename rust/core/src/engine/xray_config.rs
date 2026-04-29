//! Build the JSON configuration consumed by `xray-core` from our typed
//! [`AppConfig`].
//!
//! In the new architecture xray-core is a *transparent* SOCKS5 → VLESS
//! proxy used as an unmanaged Pluggable Transport for Arti (and, when
//! `skip_arti=true`, directly by hev-socks5-tunnel). It therefore needs:
//!
//! * **inbound** — a single SOCKS5 listener on `127.0.0.1:<dyn>`. Every
//!   CONNECT lands here.
//! * **outbound (`proxy`)** — VLESS + Reality + (gRPC | xhttp) speaking
//!   to the upstream defined in `AppConfig::outbound`.
//! * **outbound (`block`)** — sinkhole for unwanted traffic.
//! * **outbound (`dns-out`)** — handles intra-tunnel DNS queries via
//!   xray's own DNS subsystem (DoH).
//! * **routing** — DNS→`dns-out`, default → `proxy`. We do NOT add a
//!   `direct` rule for private IPs: when Arti dials its bridge at
//!   `127.0.0.1:9001` that connection must travel through `proxy`.
//! * **dns** — DoH resolver from `AppConfig::doh_server` plus the
//!   optional `doh_server_ip` for direct-IP bootstrap.
//!
//! The shape of the produced JSON is intentionally a `serde_json::Value`
//! tree — Xray-core's parser is forgiving and adding fields incrementally
//! is easier than maintaining a typed mirror of the entire schema.

use serde_json::{json, Value};
use std::net::SocketAddr;

use crate::config::AppConfig;

/// Build a complete xray-core configuration that exposes a SOCKS5 inbound
/// at `socks_in` and routes everything (including the bridge dial from
/// Arti) through a VLESS+Reality outbound to the upstream server.
pub fn build_direct(cfg: &AppConfig, socks_in: SocketAddr) -> Value {
    let outbound = vless_reality_outbound(cfg);
    json!({
        "log": {
            "loglevel": "warning",
        },
        "dns": dns_block(cfg),
        "inbounds": [socks_inbound(socks_in)],
        "outbounds": [
            outbound,
            { "tag": "block",  "protocol": "blackhole" },
            { "tag": "dns-out", "protocol": "dns" },
        ],
        "routing": routing_block(),
    })
}

fn socks_inbound(addr: SocketAddr) -> Value {
    json!({
        "tag": "socks-in",
        "protocol": "socks",
        "listen": addr.ip().to_string(),
        "port": addr.port(),
        "settings": { "auth": "noauth", "udp": true },
        "sniffing": { "enabled": true, "destOverride": ["http", "tls"] },
    })
}

fn vless_reality_outbound(cfg: &AppConfig) -> Value {
    let ob = &cfg.outbound;
    let stream = match ob.network.as_str() {
        "xhttp" => json!({
            "network": "xhttp",
            "security": "reality",
            "realitySettings": reality_settings(cfg),
            // No xhttpSettings tweaks — defaults match what the
            // server emits today.
        }),
        // Default to gRPC for the older schema.
        _ => json!({
            "network": "grpc",
            "security": "reality",
            "grpcSettings": { "serviceName": ob.grpc_service_name },
            "realitySettings": reality_settings(cfg),
        }),
    };
    json!({
        "tag": "proxy",
        "protocol": "vless",
        "settings": {
            "vnext": [{
                "address": ob.address,
                "port": ob.port,
                "users": [{
                    "id": ob.user_id,
                    "encryption": "none",
                    "flow": ob.flow,
                }],
            }],
        },
        "streamSettings": stream,
    })
}

fn reality_settings(cfg: &AppConfig) -> Value {
    let r = &cfg.outbound.reality;
    json!({
        "show": false,
        "fingerprint": r.fingerprint,
        "serverName": r.server_name,
        "publicKey": r.public_key,
        "shortId": r.short_id,
        "spiderX": "",
    })
}

fn dns_block(cfg: &AppConfig) -> Value {
    // The first server is what the DNS-out outbound forwards queries
    // through (DoH URL). The optional `doh_server_ip` is treated as a
    // bootstrap A record so xray can resolve the DoH host without
    // bouncing the lookup back through itself. `localhost` provides a
    // last-resort fallback for the daemon's own internal lookups.
    let mut servers: Vec<Value> = vec![Value::String(cfg.doh_server.clone())];
    if let Some(ip) = &cfg.doh_server_ip {
        servers.push(Value::String(ip.to_string()));
    }
    servers.push(Value::String("localhost".into()));
    json!({ "servers": servers })
}

fn routing_block() -> Value {
    json!({
        "domainStrategy": "AsIs",
        "rules": [
            // DNS hijack: any query to UDP/TCP-53 from the SOCKS inbound
            // is handed to xray's `dns-out` outbound, which uses the
            // `dns.servers` block (DoH).
            {
                "type": "field",
                "inboundTag": ["socks-in"],
                "port": 53,
                "outboundTag": "dns-out"
            },
            // Default: everything else goes through the VLESS proxy. We
            // intentionally don't define a `direct` outbound here — when
            // Arti is using xray as a Pluggable Transport, its bridge
            // CONNECT to `127.0.0.1:9001` must be tunnelled, not handled
            // locally.
            { "type": "field", "network": "tcp,udp", "outboundTag": "proxy" },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cfg(network: &str) -> AppConfig {
        let json = format!(
            r#"{{
              "bridge_rsa_id": "715213AEA5BBE71AB2E9E1AFEE02D0170206021F",
              "bridge_ed25519_id": "Rq4fdFNepS2oTFnyNrQon9FDWi46m5OFZKAGVFmMe9I",
              "doh_server": "https://dns.google/dns-query",
              "outbounds": [{{
                "tag": "proxy", "protocol": "vless",
                "settings": {{ "vnext": [{{
                  "address": "1.2.3.4", "port": 443,
                  "users": [{{ "id": "uuid-1", "flow": "" }}]
                }}] }},
                "streamSettings": {{
                  "network": "{network}",
                  {grpc}
                  "security": "reality",
                  "realitySettings": {{
                    "serverName": "example.com",
                    "publicKey": "pk",
                    "shortId": "sid",
                    "fingerprint": "qq"
                  }}
                }}
              }}]
            }}"#,
            network = network,
            grpc = if network == "grpc" {
                r#""grpcSettings": { "serviceName": "g" },"#
            } else {
                ""
            }
        );
        AppConfig::from_json(&json).unwrap()
    }

    #[test]
    fn direct_grpc_round_trips_essential_fields() {
        let cfg = sample_cfg("grpc");
        let v = build_direct(&cfg, "127.0.0.1:10808".parse().unwrap());
        let proxy = v["outbounds"][0].clone();
        assert_eq!(proxy["protocol"], "vless");
        assert_eq!(proxy["streamSettings"]["network"], "grpc");
        assert_eq!(
            proxy["streamSettings"]["grpcSettings"]["serviceName"],
            "g",
        );
        assert_eq!(proxy["streamSettings"]["realitySettings"]["serverName"], "example.com");
        assert_eq!(v["inbounds"][0]["port"], 10808);
        // First rule is DNS hijack -> dns-out, then default -> proxy.
        assert_eq!(v["routing"]["rules"][0]["outboundTag"], "dns-out");
        assert_eq!(v["routing"]["rules"][0]["port"], 53);
        assert_eq!(v["routing"]["rules"][1]["outboundTag"], "proxy");
        // dns-out outbound must exist alongside the proxy/block trio.
        let tags: Vec<String> = v["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["tag"].as_str().unwrap().to_string())
            .collect();
        assert!(tags.contains(&"dns-out".to_string()));
        assert!(tags.contains(&"proxy".to_string()));
        assert!(tags.contains(&"block".to_string()));
        // Critical: no `direct` outbound — Arti's bridge dial to
        // 127.0.0.1:9001 must travel through `proxy`.
        assert!(!tags.contains(&"direct".to_string()));
    }

    #[test]
    fn direct_xhttp_omits_grpc_settings() {
        let cfg = sample_cfg("xhttp");
        let v = build_direct(&cfg, "127.0.0.1:10808".parse().unwrap());
        let stream = &v["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "xhttp");
        assert!(stream.get("grpcSettings").is_none());
        assert_eq!(stream["realitySettings"]["publicKey"], "pk");
    }

    #[test]
    fn dns_block_includes_doh_ip_when_present() {
        let mut cfg = sample_cfg("xhttp");
        cfg.doh_server_ip = Some("8.8.8.8".parse().unwrap());
        let v = build_direct(&cfg, "127.0.0.1:10808".parse().unwrap());
        let servers: Vec<String> = v["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        assert!(servers.contains(&"https://dns.google/dns-query".to_string()));
        assert!(servers.contains(&"8.8.8.8".to_string()));
    }
}
