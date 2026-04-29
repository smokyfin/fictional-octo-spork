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
///
/// `pt_socks` is the SOCKS5 endpoint of an *unmanaged* Pluggable Transport
/// (xray-core's local socks-inbound). When the supplied [`AppConfig`]
/// carries non-empty bridge identity strings, the engine wires Arti to
/// reach the server-side Tor ORPort (`127.0.0.1:9001` per project spec)
/// through that proxy. When the bridge fields are empty, Arti bootstraps
/// against the public Tor network as a fallback.
///
/// `arti_socks` is the SOCKS5 endpoint our local proxy will *expose* to
/// hev-socks5-tunnel — i.e. where the TUN side hands its CONNECT
/// requests.
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
    //
    // Arti reaches the server's Tor ORPort (`127.0.0.1:9001` per project
    // spec) by dialling an *unmanaged* Pluggable Transport — the local
    // xray-core socks-inbound (`pt_socks`). xray's outbound carries that
    // SOCKS CONNECT request through a VLESS+Reality tunnel to the VPN
    // server; the server-side xray-core then hands the bytes to its
    // local Tor relay. From Arti's perspective the bridge appears to
    // live at `127.0.0.1:9001`, but the actual bytes flow over Reality.
    //
    // We tag the unmanaged transport `vless-pt` — the name is purely
    // local; what matters is that both the bridge and the transport
    // entry agree on it.
    if !cfg.bridge_rsa_id.is_empty() && !cfg.bridge_ed25519_id.is_empty() {
        use std::str::FromStr;
        builder
            .bridges()
            .enabled(arti_client::config::BoolOrAuto::Explicit(true));

        let mut transport = arti_client::config::pt::TransportConfigBuilder::default();
        transport
            .protocols(vec![tor_linkspec::PtTransportName::from_str("vless-pt")
                .map_err(|e| anyhow::anyhow!("PtTransportName: {e}"))?])
            .proxy_addr(pt_socks.addr);
        builder.bridges().transports().push(transport);

        let mut bridge = arti_client::config::BridgeConfigBuilder::default();
        bridge.transport("vless-pt");
        // Per spec the bridge sits behind xray on the server at
        // 127.0.0.1:9001. The "address" we hand Arti is what xray will
        // see in the SOCKS CONNECT request from Arti.
        let bridge_sock: std::net::SocketAddr = "127.0.0.1:9001"
            .parse()
            .map_err(|e| anyhow::anyhow!("bridge addr: {e}"))?;
        bridge.set_addrs(vec![tor_linkspec::BridgeAddr::new_addr_from_sockaddr(
            bridge_sock,
        )]);
        bridge.set_ids(vec![
            cfg.bridge_rsa_id
                .parse()
                .map_err(|e| anyhow::anyhow!("bridge_rsa_id: {e}"))?,
            cfg.bridge_ed25519_id
                .parse()
                .map_err(|e| anyhow::anyhow!("bridge_ed25519_id: {e}"))?,
        ]);
        builder.bridges().bridges().push(bridge);

        info!(
            transport = "vless-pt",
            proxy = %pt_socks.addr,
            bridge = "127.0.0.1:9001",
            "configured Arti to reach the server bridge via xray-PT"
        );
    } else {
        // Bridge identity not supplied — fall back to direct Tor
        // bootstrap against the public network. Useful for testing /
        // dev environments where the VLESS server is reachable but
        // bridges aren't required.
        let _ = pt_socks;
        builder
            .bridges()
            .enabled(arti_client::config::BoolOrAuto::Explicit(false));
        warn!(
            "no bridge_rsa_id / bridge_ed25519_id in config — \
             bootstrapping Arti against the public Tor network"
        );
    }

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

/// Read the SOCKS5 method-selection message and answer either with the
/// no-auth method (if the client offered it) or with `username/password`.
///
/// Xray-core's SOCKS outbound dials no-auth by default, while Leaf's SOCKS
/// outbound (back when we used it) dialed user/pass. We accept either so
/// the same Arti listener can serve both.
async fn socks5_handshake(sock: &mut TcpStream, auth: &(String, String)) -> Result<()> {
    let mut header = [0u8; 2];
    sock.read_exact(&mut header).await.map_err(Error::from)?;
    if header[0] != 0x05 {
        return Err(Error::Other("not a SOCKS5 client".into()));
    }
    let nmethods = header[1] as usize;
    let mut methods = vec![0u8; nmethods];
    sock.read_exact(&mut methods).await.map_err(Error::from)?;

    if methods.contains(&0x00) {
        // Accept no-auth and skip the username/password subnegotiation
        // entirely. Used by Xray's `socks` outbound.
        sock.write_all(&[0x05, 0x00]).await.map_err(Error::from)?;
        let _ = auth;
        return Ok(());
    }
    if !methods.contains(&0x02) {
        // Neither no-auth nor user/pass — reject the connection.
        sock.write_all(&[0x05, 0xff]).await.map_err(Error::from)?;
        return Err(Error::Other(
            "client offered no acceptable SOCKS5 method".into(),
        ));
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
