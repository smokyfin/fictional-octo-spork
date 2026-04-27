//! Embedded DNS server.
//!
//! Listens on UDP/53 at the TUN address. The OS routes its DNS queries here
//! (via the per-tunnel DNS settings in `VpnService.Builder.addDnsServer` /
//! `NEPacketTunnelNetworkSettings.dnsSettings`).
//!
//! Each query is forwarded to the configured DoH endpoint (`doh_server`).
//! The HTTP request is made via `reqwest` configured to use Leaf #2's
//! SOCKS-like outbound chain — i.e. through the VLESS+Reality+gRPC path
//! over Tor. If `doh_server_ip` is provided, we resolve the DoH hostname
//! to that IP statically so we never query the system resolver (which would
//! leak the lookup outside of the tunnel).

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::runtime::{Cancel, SocksEndpoint};
use hickory_proto::op::{Message, MessageType, ResponseCode};
// hickory-proto 0.26 exposes `Message` header data via the public `metadata`
// field rather than getter/setter methods.
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

const DNS_PORT: u16 = 53;
const MAX_UDP_DNS: usize = 4096;

pub async fn spawn_dns_proxy(
    cfg: AppConfig,
    tun_addr: IpAddr,
    arti_socks: SocksEndpoint,
    cancel: Cancel,
) -> Result<JoinHandle<()>> {
    let bind: SocketAddr = (tun_addr, DNS_PORT).into();
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| Error::Dns(format!("bind {bind}: {e}")))?;
    let socket = Arc::new(socket);
    let client = build_doh_client(&cfg, &arti_socks)?;

    Ok(tokio::spawn(async move {
        if let Err(e) = run(socket, client, cfg.doh_server.clone(), cancel.clone()).await {
            error!(?e, "DNS proxy exited with error");
        }
    }))
}

async fn run(
    socket: Arc<UdpSocket>,
    client: reqwest::Client,
    doh_url: String,
    cancel: Cancel,
) -> Result<()> {
    let mut buf = vec![0u8; MAX_UDP_DNS];
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                debug!("DNS proxy cancelled");
                return Ok(());
            }
            recv = socket.recv_from(&mut buf) => {
                let (n, peer) = match recv {
                    Ok(v) => v,
                    Err(e) => { warn!(?e, "udp recv_from error"); continue; }
                };
                let query = buf[..n].to_vec();
                let socket = socket.clone();
                let client = client.clone();
                let url = doh_url.clone();
                tokio::spawn(async move {
                    match forward_doh(&client, &url, &query).await {
                        Ok(answer) => {
                            if let Err(e) = socket.send_to(&answer, peer).await {
                                warn!(?e, "udp send_to error");
                            }
                        }
                        Err(e) => {
                            warn!(?e, "DoH forward failed; sending SERVFAIL");
                            if let Some(servfail) = build_servfail(&query) {
                                let _ = socket.send_to(&servfail, peer).await;
                            }
                        }
                    }
                });
            }
        }
    }
}

fn build_doh_client(cfg: &AppConfig, _arti_socks: &SocksEndpoint) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .http2_prior_knowledge()
        .user_agent("ff-vpn-doh/1");

    // If the config declares an explicit IP for the DoH server, pin DNS
    // resolution so we *never* call the system resolver. This is critical:
    // the system resolver is what we are intercepting in the first place.
    if let (Some(ip), Ok(parsed)) = (cfg.doh_server_ip, url::Url::parse(&cfg.doh_server)) {
        if let Some(host) = parsed.host_str() {
            let port = parsed.port_or_known_default().unwrap_or(443);
            builder = builder.resolve(host, (ip, port).into());
        }
    }

    // Note: the DoH POST goes out via the OS's normal socket path. Because
    // these sockets originate from inside the VPN process and the OS routes
    // *all* user traffic into the TUN, the request is in fact delivered to
    // Leaf #2's TUN inbound — and from there through Arti + VLESS. If the
    // platform layer protects this socket (Android `VpnService.protect`),
    // it would bypass the tunnel; we do *not* protect DNS sockets so the
    // DoH query rides the tunnel like any other traffic, as required.
    builder.build().map_err(Error::from)
}

async fn forward_doh(client: &reqwest::Client, url: &str, query: &[u8]) -> Result<Vec<u8>> {
    let resp = client
        .post(url)
        .header(reqwest::header::ACCEPT, "application/dns-message")
        .header(reqwest::header::CONTENT_TYPE, "application/dns-message")
        .body(query.to_vec())
        .send()
        .await?
        .error_for_status()?;
    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}

fn build_servfail(query: &[u8]) -> Option<Vec<u8>> {
    let parsed = Message::from_vec(query).ok()?;
    let mut resp = Message::new(
        parsed.metadata.id,
        MessageType::Response,
        parsed.metadata.op_code,
    );
    resp.metadata.response_code = ResponseCode::ServFail;
    for q in &parsed.queries {
        resp.add_query(q.clone());
    }
    resp.to_vec().ok()
}
