//! Build the JSON configuration consumed by `xray-core` from our typed
//! [`AppConfig`].
//!
//! Xray-core has a *much* richer config schema than our `AppConfig`; we
//! only emit the slice we actually need:
//!
//! * **inbound** — a single SOCKS5 listener on `127.0.0.1:<dyn>`. The
//!   tun2socks engine pumps every IP packet that crosses the TUN here.
//! * **outbound (`proxy`)** — VLESS + Reality + (gRPC | xhttp) speaking to
//!   the upstream defined in `AppConfig::outbound`.
//! * **outbound (`direct`)** — needed by the routing table for
//!   private-IP / loopback traffic.
//! * **outbound (`block`)** — sinkhole for things that must never leave.
//! * **routing** — `geoip:private` → `direct`, default → `proxy`.
//! * **dns** — DoH resolver from `AppConfig::doh_server` (e.g.
//!   `https://dns.google/dns-query`); fallback to the bundled IP if the
//!   name itself can't be resolved.
//!
//! For the "via Tor" path we expose [`add_arti_chain`] which prepends an
//! upstream SOCKS5 outbound (pointing at the Arti listener) and rewrites
//! the `proxy` outbound's chain so the VLESS dial happens over Tor.
//!
//! The shape of the produced JSON is intentionally a `serde_json::Value`
//! tree — Xray-core's parser is forgiving and adding fields incrementally
//! is much easier than maintaining a typed mirror of the entire schema.

use serde_json::{json, Value};
use std::net::SocketAddr;

use crate::config::AppConfig;

/// Build a complete xray-core configuration that exposes a SOCKS5 inbound
/// at `socks_in` and routes everything except private IPs through the
/// upstream defined by `cfg`.
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
            { "tag": "direct", "protocol": "freedom" },
            { "tag": "block",  "protocol": "blackhole" },
            { "tag": "dns-out", "protocol": "dns" },
        ],
        "routing": routing_block(),
    })
}

/// Re-issue the configuration produced by [`build_direct`] but force every
/// connection from the `proxy` outbound to first traverse the SOCKS5
/// listener at `arti_socks` (i.e. through Arti / Tor) before reaching the
/// VLESS server.
///
/// We use Xray's `streamSettings.sockopt.dialerProxy` — a per-outbound
/// override that hands the dial to another *outbound* tag, which we add
/// here as `arti-socks`.
pub fn build_via_tor(cfg: &AppConfig, socks_in: SocketAddr, arti_socks: SocketAddr) -> Value {
    let mut proxy = vless_reality_outbound(cfg);
    let stream = proxy
        .get_mut("streamSettings")
        .expect("vless outbound always carries streamSettings");
    stream
        .as_object_mut()
        .expect("streamSettings is an object")
        .insert(
            "sockopt".into(),
            json!({ "dialerProxy": "arti-socks" }),
        );
    json!({
        "log": { "loglevel": "warning" },
        "dns": dns_block(cfg),
        "inbounds": [socks_inbound(socks_in)],
        "outbounds": [
            proxy,
            arti_socks_outbound(arti_socks),
            { "tag": "direct", "protocol": "freedom" },
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

fn arti_socks_outbound(addr: SocketAddr) -> Value {
    json!({
        "tag": "arti-socks",
        "protocol": "socks",
        "settings": {
            "servers": [{
                "address": addr.ip().to_string(),
                "port": addr.port(),
            }],
        },
    })
}

fn dns_block(cfg: &AppConfig) -> Value {
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
            // Catch every DNS query (clients dialling 10.10.0.2:53 or any
            // UDP/TCP port 53 destination) and resolve it through xray's
            // built-in DNS subsystem (which honours the `dns.servers` block
            // — DoH `dns.google` in our config). Without this rule the
            // queries would be sent verbatim to the unreachable TUN
            // address and time out.
            {
                "type": "field",
                "inboundTag": ["socks-in"],
                "port": 53,
                "outboundTag": "dns-out"
            },
            // Loopback / RFC1918 / link-local traffic stays on the
            // device — never tunnelled.
            { "type": "field", "ip": ["geoip:private"], "outboundTag": "direct" },
            // Default: everything else goes through proxy.
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
              "bridge_rsa_id": "x",
              "bridge_ed25519_id": "y",
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
        // First rule is DNS hijack -> dns-out, then private IPs ->
        // direct, default -> proxy.
        assert_eq!(v["routing"]["rules"][0]["outboundTag"], "dns-out");
        assert_eq!(v["routing"]["rules"][0]["port"], 53);
        assert_eq!(v["routing"]["rules"][1]["outboundTag"], "direct");
        assert_eq!(v["routing"]["rules"][2]["outboundTag"], "proxy");
        // dns-out outbound must exist alongside the proxy/direct/block trio.
        let tags: Vec<String> = v["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["tag"].as_str().unwrap().to_string())
            .collect();
        assert!(tags.contains(&"dns-out".to_string()));
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
    fn via_tor_inserts_arti_outbound_and_dialer_proxy() {
        let cfg = sample_cfg("xhttp");
        let v = build_via_tor(
            &cfg,
            "127.0.0.1:10808".parse().unwrap(),
            "127.0.0.1:9150".parse().unwrap(),
        );
        let proxy = &v["outbounds"][0];
        assert_eq!(
            proxy["streamSettings"]["sockopt"]["dialerProxy"],
            "arti-socks",
        );
        let arti = &v["outbounds"][1];
        assert_eq!(arti["tag"], "arti-socks");
        assert_eq!(arti["protocol"], "socks");
        assert_eq!(arti["settings"]["servers"][0]["port"], 9150);
    }
}
