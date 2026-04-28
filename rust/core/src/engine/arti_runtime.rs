//! Arti runtime — Tor client + SOCKS5 listener.
//!
//! We don't embed Arti's CLI binary. Instead, we use `arti_client` to:
//!
//!   1. Build a `TorClientConfig` with the user's bridge configured as an
//!      *unmanaged* PT — the PT's `proxy_addr` points at Leaf #1's SOCKS5
//!      listener.
//!   2. Bootstrap a `TorClient<TokioRustlsRuntime>`.
//!   3. Run a small SOCKS5 server (random port + random auth) that, on every
//!      accepted connection, opens a Tor stream via `client.connect(target)`
//!      and bidirectionally pumps bytes.
//!
//! Leaf #2 then talks to *that* SOCKS5 server as the outermost link in its
//! chain, so the user's traffic flows:
//!
//!   `user → TUN → Leaf #2 → arti_runtime SOCKS → TorClient → bridge (PT) → Tor → exit`.

use std::path::PathBuf;

use anyhow::Context as _;
use arti_client::{TorClient, TorClientConfig};
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tor_rtcompat::PreferredRuntime;
use tracing::{debug, error, info, warn};

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::runtime::{Cancel, SocksEndpoint};

/// Spawn the whole Arti subsystem and return the join handle.
pub async fn spawn_arti(
    cfg: AppConfig,
    pt_socks: SocksEndpoint,
    arti_socks: SocksEndpoint,
    private_dir: PathBuf,
    cancel: Cancel,
) -> Result<JoinHandle<()>> {
    let task = tokio::spawn(async move {
        if let Err(e) = run(cfg, pt_socks, arti_socks, private_dir, cancel.clone()).await {
            error!(?e, "Arti task exited with error");
        }
        cancel.cancel();
    });
    Ok(task)
}

async fn run(
    cfg: AppConfig,
    pt_socks: SocksEndpoint,
    arti_socks: SocksEndpoint,
    private_dir: PathBuf,
    cancel: Cancel,
) -> Result<()> {
    let cache_dir = private_dir.join("arti-cache");
    let state_dir = private_dir.join("arti-state");
    std::fs::create_dir_all(&cache_dir).map_err(Error::from)?;
    std::fs::create_dir_all(&state_dir).map_err(Error::from)?;

    let arti_cfg = build_arti_config(&cfg, &pt_socks, &cache_dir, &state_dir)
        .map_err(|e| Error::Arti(e.to_string()))?;

    info!("bootstrapping Arti TorClient");
    let mut client = TorClient::with_runtime(
        PreferredRuntime::current().map_err(|e| Error::Arti(e.to_string()))?,
    )
    .config(arti_cfg)
    .create_bootstrapped()
    .await
    .map_err(|e| Error::Arti(format!("bootstrap: {e}")))?;
    info!("Arti bootstrapped");

    // Apply UI-driven exit-country preference, if any.
    if let Some(code) = cfg.user.exit_country.as_deref() {
        let parsed = code.parse::<tor_geoip::CountryCode>();
        match parsed {
            Ok(cc) => {
                let mut prefs = arti_client::StreamPrefs::new();
                prefs.exit_country(cc);
                client.set_stream_prefs(prefs);
            }
            Err(e) => warn!(?e, code, "ignoring invalid exit-country code"),
        }
    }
    // Re-bind as immutable for the loop below.
    let client = client;

    let listener = TcpListener::bind(arti_socks.addr)
        .await
        .map_err(Error::from)?;
    info!(addr = %arti_socks.addr, "Arti SOCKS5 listener up");

    let auth = (arti_socks.username.clone(), arti_socks.password.clone());
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Arti runtime cancelled");
                return Ok(());
            }
            accept = listener.accept() => {
                let (sock, _) = match accept {
                    Ok(v) => v,
                    Err(e) => { warn!(?e, "accept error"); continue; }
                };
                let client = client.clone();
                let cancel = cancel.clone();
                let auth = auth.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_socks_conn(sock, client, auth, cancel).await {
                        debug!(?e, "socks connection ended");
                    }
                });
            }
        }
    }
}

fn build_arti_config(
    cfg: &AppConfig,
    pt_socks: &SocksEndpoint,
    cache_dir: &std::path::Path,
    state_dir: &std::path::Path,
) -> anyhow::Result<TorClientConfig> {
    let mut builder = TorClientConfig::builder();
    builder
        .storage()
        .cache_dir(arti_client::config::CfgPath::new(
            cache_dir.display().to_string(),
        ))
        .state_dir(arti_client::config::CfgPath::new(
            state_dir.display().to_string(),
        ));

    // ---- Bridges ---------------------------------------------------------
    // Format: `Bridge obfs4 <addr>:<port> <RSA-id> ed25519:<ed25519-id>`
    // We point the bridge at a sentinel address; the *unmanaged* obfs4
    // transport below carries the real connection.
    let bridge_line = format!(
        "Bridge obfs4 0.0.0.0:1 {rsa} ed25519:{ed}",
        rsa = cfg.bridge_rsa_id,
        ed = cfg.bridge_ed25519_id,
    );
    let bridge: arti_client::config::BridgeConfigBuilder = bridge_line
        .parse()
        .with_context(|| format!("invalid bridge line: {bridge_line:?}"))?;
    builder.bridges().bridges().push(bridge);

    // ---- Pluggable Transport (unmanaged, SOCKS5) ------------------------
    // `proxy_addr` is the SOCKS5 endpoint Leaf #1 is listening on.
    {
        let mut transport = arti_client::config::pt::TransportConfigBuilder::default();
        let proto: tor_linkspec::PtTransportName = "obfs4".parse()?;
        transport.protocols(vec![proto]);
        transport.proxy_addr(pt_socks.addr);
        builder.bridges().transports().push(transport);
    }

    builder
        .bridges()
        .enabled(arti_client::config::BoolOrAuto::Explicit(true));

    Ok(builder.build()?)
}

// ---- Minimal SOCKS5 server -----------------------------------------------

/// Handle one client of our local SOCKS5 listener: parse SOCKS5 + auth, dial
/// the requested target through Arti, then byte-pump until EOF.
async fn handle_socks_conn(
    mut sock: TcpStream,
    client: TorClient<PreferredRuntime>,
    auth: (String, String),
    cancel: Cancel,
) -> Result<()> {
    socks5_handshake(&mut sock, &auth).await?;
    let target = socks5_read_request(&mut sock).await?;

    debug!(?target, "dialing through Tor");
    let stream = client
        .connect(target.clone())
        .await
        .map_err(|e| Error::Arti(format!("connect {target:?}: {e}")))?;

    socks5_send_reply_success(&mut sock).await?;

    let (mut sr, mut sw) = sock.into_split();
    let (mut tr, mut tw) = tokio::io::split(stream);
    let cancel_a = cancel.clone();
    let a = tokio::spawn(async move {
        tokio::select! {
            _ = cancel_a.cancelled() => {}
            _ = tokio::io::copy(&mut sr, &mut tw) => {}
        }
    });
    let cancel_b = cancel.clone();
    let b = tokio::spawn(async move {
        tokio::select! {
            _ = cancel_b.cancelled() => {}
            _ = tokio::io::copy(&mut tr, &mut sw) => {}
        }
    });
    let _ = a.await;
    let _ = b.await;
    Ok(())
}

/// Read the SOCKS5 method-selection message and answer with `username/password`.
async fn socks5_handshake(sock: &mut TcpStream, auth: &(String, String)) -> Result<()> {
    let mut header = [0u8; 2];
    sock.read_exact(&mut header).await.map_err(Error::from)?;
    if header[0] != 0x05 {
        return Err(Error::Other("not a SOCKS5 client".into()));
    }
    let nmethods = header[1] as usize;
    let mut methods = vec![0u8; nmethods];
    sock.read_exact(&mut methods).await.map_err(Error::from)?;

    if !methods.contains(&0x02) {
        // Reject — we require user/pass.
        sock.write_all(&[0x05, 0xff]).await.map_err(Error::from)?;
        return Err(Error::Other("client refused user/pass auth".into()));
    }
    sock.write_all(&[0x05, 0x02]).await.map_err(Error::from)?;

    // Subnegotiation: 1 ver, 1 ulen, ulen, 1 plen, plen.
    let ver = sock.read_u8().await.map_err(Error::from)?;
    if ver != 0x01 {
        return Err(Error::Other("bad auth subnegotiation version".into()));
    }
    let ulen = sock.read_u8().await.map_err(Error::from)? as usize;
    let mut user = vec![0u8; ulen];
    sock.read_exact(&mut user).await.map_err(Error::from)?;
    let plen = sock.read_u8().await.map_err(Error::from)? as usize;
    let mut pass = vec![0u8; plen];
    sock.read_exact(&mut pass).await.map_err(Error::from)?;

    let ok = crate::util::ct_eq(&user, auth.0.as_bytes())
        && crate::util::ct_eq(&pass, auth.1.as_bytes());
    sock.write_all(&[0x01, if ok { 0x00 } else { 0x01 }])
        .await
        .map_err(Error::from)?;
    if !ok {
        return Err(Error::Other("invalid SOCKS5 credentials".into()));
    }
    Ok(())
}

/// Read a SOCKS5 CONNECT request and return `host:port`.
async fn socks5_read_request(sock: &mut TcpStream) -> Result<arti_client::TorAddr> {
    let mut head = [0u8; 4];
    sock.read_exact(&mut head).await.map_err(Error::from)?;
    let (ver, cmd, _rsv, atyp) = (head[0], head[1], head[2], head[3]);
    if ver != 0x05 || cmd != 0x01 {
        return Err(Error::Other(format!("unsupported SOCKS5 request {head:?}")));
    }
    let (host, port) = match atyp {
        0x01 => {
            // IPv4
            let mut bytes = [0u8; 4 + 2];
            sock.read_exact(&mut bytes).await.map_err(Error::from)?;
            let ip = std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
            let port = u16::from_be_bytes([bytes[4], bytes[5]]);
            (ip.to_string(), port)
        }
        0x03 => {
            // Domain
            let mut len = [0u8; 1];
            sock.read_exact(&mut len).await.map_err(Error::from)?;
            let mut buf = BytesMut::with_capacity(len[0] as usize + 2);
            buf.resize(len[0] as usize + 2, 0);
            sock.read_exact(&mut buf).await.map_err(Error::from)?;
            let host = String::from_utf8(buf[..len[0] as usize].to_vec())
                .map_err(|e| Error::Other(format!("bad host: {e}")))?;
            let port = u16::from_be_bytes([buf[len[0] as usize], buf[len[0] as usize + 1]]);
            (host, port)
        }
        0x04 => {
            // IPv6
            let mut bytes = [0u8; 16 + 2];
            sock.read_exact(&mut bytes).await.map_err(Error::from)?;
            let mut a = [0u8; 16];
            a.copy_from_slice(&bytes[..16]);
            let ip = std::net::Ipv6Addr::from(a);
            let port = u16::from_be_bytes([bytes[16], bytes[17]]);
            (ip.to_string(), port)
        }
        other => return Err(Error::Other(format!("unsupported atyp {other}"))),
    };

    arti_client::TorAddr::from((host.as_str(), port))
        .map_err(|e| Error::Other(format!("invalid TorAddr: {e}")))
}

async fn socks5_send_reply_success(sock: &mut TcpStream) -> Result<()> {
    // VER=5, REP=0 (succeeded), RSV=0, ATYP=IPv4, BND.ADDR=0.0.0.0, BND.PORT=0
    sock.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(Error::from)?;
    Ok(())
}
