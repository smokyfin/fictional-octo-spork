//! Configuration parser for the VPN client.
//!
//! Source format follows `https://incss.ru/vless.conf` (Xray/VLESS-compatible
//! JSON). Only the explicitly-allowlisted fields are extracted — everything
//! else (`routing`, `inbounds`, etc.) is intentionally ignored because the
//! TUN inbound and routing rules are produced internally.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;

use crate::error::{Error, Result};

/// Top-level configuration the engine consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub bridge_rsa_id: String,
    pub bridge_ed25519_id: String,
    pub doh_server: String,
    pub doh_server_ip: Option<IpAddr>,
    pub outbound: VlessOutbound,

    /// Optional UI-driven knobs (country picker, per-app routing, etc.).
    #[serde(default)]
    pub user: UserPrefs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPrefs {
    /// 2-letter ISO country code for Tor exit node, e.g. "DE". Empty means any.
    #[serde(default)]
    pub exit_country: Option<String>,

    /// Android per-app routing. If `Some(allowlist)`, only these package names
    /// go through the VPN. If `None`, all apps go through (default).
    #[serde(default)]
    pub allowed_packages: Option<Vec<String>>,

    /// Android per-app routing — packages explicitly excluded from the VPN.
    #[serde(default)]
    pub disallowed_packages: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VlessOutbound {
    pub tag: String,
    pub address: String,
    pub port: u16,
    pub user_id: String,
    #[serde(default)]
    pub flow: String,
    /// VLESS stream transport: `grpc` or `xhttp`. Older configs without
    /// this field default to `grpc` for backward compatibility.
    #[serde(default = "default_network")]
    pub network: String,
    /// Only present when `network == "grpc"`. Empty string for `xhttp`.
    #[serde(default)]
    pub grpc_service_name: String,
    pub reality: RealitySettings,
}

fn default_network() -> String {
    "grpc".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealitySettings {
    pub server_name: String,
    pub public_key: String,
    pub short_id: String,
    pub fingerprint: String,
}

// ---- Raw JSON shape -------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawRoot {
    bridge_rsa_id: String,
    bridge_ed25519_id: String,
    doh_server: String,
    #[serde(default)]
    doh_server_ip: Option<String>,
    outbounds: Vec<RawOutbound>,
}

#[derive(Debug, Deserialize)]
struct RawOutbound {
    #[serde(default)]
    tag: String,
    protocol: String,
    settings: RawOutboundSettings,
    #[serde(rename = "streamSettings")]
    stream_settings: RawStreamSettings,
}

#[derive(Debug, Deserialize)]
struct RawOutboundSettings {
    vnext: Vec<RawVnext>,
}

#[derive(Debug, Deserialize)]
struct RawVnext {
    address: String,
    port: u16,
    users: Vec<RawUser>,
}

#[derive(Debug, Deserialize)]
struct RawUser {
    id: String,
    #[serde(default)]
    flow: String,
}

#[derive(Debug, Deserialize)]
struct RawStreamSettings {
    network: String,
    #[serde(default, rename = "grpcSettings")]
    grpc_settings: Option<RawGrpc>,
    security: String,
    #[serde(default, rename = "realitySettings")]
    reality_settings: Option<RawReality>,
}

#[derive(Debug, Deserialize)]
struct RawGrpc {
    #[serde(rename = "serviceName")]
    service_name: String,
}

#[derive(Debug, Deserialize)]
struct RawReality {
    #[serde(rename = "serverName")]
    server_name: String,
    #[serde(rename = "publicKey")]
    public_key: String,
    #[serde(rename = "shortId")]
    short_id: String,
    #[serde(rename = "fingerprint")]
    fingerprint: String,
}

impl AppConfig {
    /// Parse the upstream JSON, keeping only the fields the engine cares about.
    pub fn from_json(text: &str) -> Result<Self> {
        let raw: RawRoot = serde_json::from_str(text)?;
        let outbound = raw
            .outbounds
            .into_iter()
            .find(|o| o.protocol.eq_ignore_ascii_case("vless"))
            .ok_or_else(|| Error::InvalidConfig("no vless outbound found".into()))?;

        if !outbound
            .stream_settings
            .security
            .eq_ignore_ascii_case("reality")
        {
            return Err(Error::InvalidConfig(
                "only reality streamSettings.security is supported".into(),
            ));
        }
        let network_lc = outbound.stream_settings.network.to_ascii_lowercase();
        if network_lc != "grpc" && network_lc != "xhttp" {
            return Err(Error::InvalidConfig(format!(
                "only grpc/xhttp streamSettings.network are supported (got {})",
                outbound.stream_settings.network
            )));
        }
        // gRPC carries a `serviceName` we record verbatim; `xhttp` has no
        // analogous identifier we need to preserve, so we leave it empty.
        let grpc_service_name = if network_lc == "grpc" {
            outbound
                .stream_settings
                .grpc_settings
                .ok_or_else(|| Error::InvalidConfig("missing grpcSettings".into()))?
                .service_name
        } else {
            String::new()
        };
        let reality = outbound
            .stream_settings
            .reality_settings
            .ok_or_else(|| Error::InvalidConfig("missing realitySettings".into()))?;
        let vnext = outbound
            .settings
            .vnext
            .into_iter()
            .next()
            .ok_or_else(|| Error::InvalidConfig("empty vnext list".into()))?;
        let user = vnext
            .users
            .into_iter()
            .next()
            .ok_or_else(|| Error::InvalidConfig("empty users list".into()))?;

        let doh_server_ip = match raw.doh_server_ip {
            Some(s) if !s.trim().is_empty() => Some(
                s.parse::<IpAddr>()
                    .map_err(|e| Error::InvalidConfig(format!("doh_server_ip: {e}")))?,
            ),
            _ => None,
        };

        Ok(AppConfig {
            bridge_rsa_id: raw.bridge_rsa_id,
            bridge_ed25519_id: raw.bridge_ed25519_id,
            doh_server: raw.doh_server,
            doh_server_ip,
            outbound: VlessOutbound {
                tag: outbound.tag,
                address: vnext.address,
                port: vnext.port,
                user_id: user.id,
                flow: user.flow,
                network: network_lc,
                grpc_service_name,
                reality: RealitySettings {
                    server_name: reality.server_name,
                    public_key: reality.public_key,
                    short_id: reality.short_id,
                    fingerprint: reality.fingerprint,
                },
            },
            user: UserPrefs::default(),
        })
    }

    /// Default upstream URL (`https://incss.ru/vless.conf`) — the UI passes
    /// this in by default but the user can override.
    pub const DEFAULT_REMOTE_URL: &'static str = "https://incss.ru/vless.conf";

    /// Fetch + parse the upstream configuration. Performed *outside* the VPN
    /// tunnel (the tunnel is not running yet).
    pub async fn fetch(url: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("ff-vpn-client/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let body = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Self::from_json(&body)
    }

    /// Persist the parsed config to disk for reuse.
    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a previously-saved parsed config.
    pub fn load(path: &PathBuf) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "bridge_rsa_id": "715213AEA5BBE71AB2E9E1AFEE02D0170206021F",
      "bridge_ed25519_id": "Rq4fdFNepS2oTFnyNrQon9FDWi46m5OFZKAGVFmMe9I",
      "doh_server": "https://dns.google/dns-query",
      "doh_server_ip": "8.8.8.8",
      "outbounds": [{
        "tag": "proxy",
        "protocol": "vless",
        "settings": {"vnext":[{"address":"217.177.47.30","port":8090,
          "users":[{"id":"ea183a5a-a968-4343-b468-7057b1384770","encryption":"none","flow":""}]}]},
        "streamSettings": {"network":"grpc","grpcSettings":{"serviceName":"grpc"},
          "security":"reality","realitySettings":{"serverName":"urentbike.ru",
            "publicKey":"aEikAoKpvBlef3fmB956k_I7X3iI2oQ032Zs3YRAiEw",
            "shortId":"6b2f4e6ac9b1d2f0","fingerprint":"qq"}}
      }]
    }"#;

    const SAMPLE_XHTTP: &str = r#"{
      "bridge_rsa_id": "9A5E28708880EB92217A937F56D640D23551F886",
      "bridge_ed25519_id": "1Vvw08iKSZVW9ghoiYBWl7qR30d5DNJu0c7EFp0XFZ4",
      "doh_server": "https://dns.google/dns-query",
      "outbounds": [{
        "tag": "proxy",
        "protocol": "vless",
        "settings": {"vnext":[{"address":"144.31.184.170","port":8090,
          "users":[{"id":"3701ba53-4573-466c-a474-f37923ce5bd1","encryption":"none","flow":""}]}]},
        "streamSettings": {"network":"xhttp",
          "security":"reality","realitySettings":{"serverName":"ads.x5.ru",
            "publicKey":"94T5KqTcnBNXDmlobpF7rmsYmPt6vqB_dWQcIi9XjAI",
            "shortId":"6b2f4e6ac9b1d2f0","fingerprint":"qq"}}
      }]
    }"#;

    #[test]
    fn parses_sample_config() {
        let cfg = AppConfig::from_json(SAMPLE).unwrap();
        assert_eq!(cfg.outbound.address, "217.177.47.30");
        assert_eq!(cfg.outbound.port, 8090);
        assert_eq!(cfg.outbound.network, "grpc");
        assert_eq!(cfg.outbound.grpc_service_name, "grpc");
        assert_eq!(cfg.outbound.reality.server_name, "urentbike.ru");
        assert_eq!(cfg.doh_server_ip.unwrap().to_string(), "8.8.8.8");
    }

    #[test]
    fn parses_xhttp_config() {
        let cfg = AppConfig::from_json(SAMPLE_XHTTP).unwrap();
        assert_eq!(cfg.outbound.address, "144.31.184.170");
        assert_eq!(cfg.outbound.network, "xhttp");
        assert_eq!(cfg.outbound.grpc_service_name, "");
        assert_eq!(cfg.outbound.reality.server_name, "ads.x5.ru");
    }

    #[test]
    fn rejects_non_vless_outbound() {
        let bad = SAMPLE.replace("\"protocol\": \"vless\"", "\"protocol\": \"trojan\"");
        assert!(AppConfig::from_json(&bad).is_err());
    }

    #[test]
    fn rejects_unsupported_network() {
        let bad = SAMPLE.replace("\"network\":\"grpc\"", "\"network\":\"tcp\"");
        assert!(AppConfig::from_json(&bad).is_err());
    }
}
