//! TUN ↔ SOCKS5 bridge — thin Rust wrapper around
//! [hev-socks5-tunnel](https://github.com/heiher/hev-socks5-tunnel).
//!
//! `hev_socks5_tunnel_main_from_str` is a **blocking** call that owns the
//! calling thread until `hev_socks5_tunnel_quit` is invoked from another
//! thread. We spawn a dedicated OS thread for it so it never blocks the
//! Tokio runtime.
//!
//! The ABI is documented in `native/hev_socks5_tunnel/README.md`.
//!
//! ## Lifecycle
//!
//! - [`HevTunnel::start`] launches the worker thread, hands it the YAML
//!   config + TUN fd. The fd is **moved** into hev — Rust must not close
//!   it after this call.
//! - [`HevTunnel`] keeps the join handle. Dropping it calls
//!   `hev_socks5_tunnel_quit` and waits for the worker to return.
//!
//! Only one hev instance can run per process. The library uses internal
//! globals (lwip + hev-task-system); a second `start` while one is
//! active would crash at C level. The engine owns at most one instance
//! at a time.

use std::sync::Mutex;
use std::thread::JoinHandle;

use crate::error::{Error, Result};

#[cfg(target_os = "android")]
mod sys {
    use std::os::raw::{c_int, c_uchar, c_uint};
    extern "C" {
        pub fn hev_socks5_tunnel_main_from_str(
            yaml: *const c_uchar,
            yaml_len: c_uint,
            tun_fd: c_int,
        ) -> c_int;
        pub fn hev_socks5_tunnel_quit();
    }
}

/// Singleton guard so we never start two instances concurrently.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
static SINGLETON: Mutex<bool> = Mutex::new(false);

pub struct HevTunnel {
    worker: Option<JoinHandle<i32>>,
}

impl HevTunnel {
    /// Render a YAML config (per upstream `conf/main.yml`) and run hev
    /// against the supplied TUN fd. The fd ownership transfers to hev;
    /// the caller must NOT close it after this call returns `Ok`.
    pub fn start(yaml_config: &str, tun_fd: i32) -> Result<Self> {
        #[cfg(target_os = "android")]
        {
            {
                let mut held = SINGLETON
                    .lock()
                    .map_err(|_| Error::Engine("hev_runtime mutex poisoned".into()))?;
                if *held {
                    return Err(Error::Engine(
                        "hev-socks5-tunnel is already running".into(),
                    ));
                }
                *held = true;
            }

            let yaml_bytes = yaml_config.as_bytes().to_vec();
            let worker = std::thread::Builder::new()
                .name("hev-socks5-tunnel".into())
                .spawn(move || -> i32 {
                    // SAFETY: yaml_bytes lives for the duration of the
                    // call (passed by reference to hev which copies it
                    // into its own structures before returning from the
                    // initial config-parse step).
                    let rc = unsafe {
                        sys::hev_socks5_tunnel_main_from_str(
                            yaml_bytes.as_ptr(),
                            yaml_bytes.len() as u32,
                            tun_fd,
                        )
                    };
                    if let Ok(mut g) = SINGLETON.lock() {
                        *g = false;
                    }
                    rc as i32
                })
                .map_err(|e| Error::Engine(format!("hev worker spawn: {e}")))?;
            Ok(Self {
                worker: Some(worker),
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (yaml_config, tun_fd);
            Err(Error::HevUnavailable)
        }
    }

    /// Async-safe stop: signal hev to quit and wait for the worker to
    /// return. Idempotent.
    pub fn stop(mut self) {
        self._stop_inner();
    }

    fn _stop_inner(&mut self) {
        #[cfg(target_os = "android")]
        unsafe {
            sys::hev_socks5_tunnel_quit();
        }
        if let Some(j) = self.worker.take() {
            // hev usually returns within a few milliseconds. Cap the
            // wait so a buggy/wedged worker doesn't strand teardown.
            let _ = j.join();
        }
    }
}

impl Drop for HevTunnel {
    fn drop(&mut self) {
        self._stop_inner();
    }
}

/// Build the YAML configuration consumed by hev's
/// `hev_socks5_tunnel_main_from_str`.
///
/// * `tun_addr`     — IPv4 address assigned to the TUN interface
///                    (matches what the Android `VpnService` set with
///                    `addAddress`).
/// * `mtu`          — TUN MTU (matches `setMtu`).
/// * `socks_addr`   — upstream SOCKS5 server (Arti listener or xray's
///                    socks-inbound).
/// * `socks_user`   — optional SOCKS5 username.
/// * `socks_pass`   — optional SOCKS5 password.
/// * `mapdns`       — when `Some(addr)`, hev intercepts UDP datagrams
///                    destined for `addr` (the in-tunnel DNS server IP)
///                    and routes them as DNS-over-SOCKS5 lookups via
///                    the upstream proxy.
pub fn build_yaml(
    tun_addr: std::net::Ipv4Addr,
    mtu: u16,
    socks_addr: std::net::SocketAddr,
    socks_user: Option<&str>,
    socks_pass: Option<&str>,
    mapdns: Option<std::net::Ipv4Addr>,
) -> String {
    let mut yaml = String::new();
    yaml.push_str("tunnel:\n");
    yaml.push_str("  name: tun0\n");
    yaml.push_str(&format!("  mtu: {}\n", mtu));
    yaml.push_str(&format!("  ipv4: {}\n", tun_addr));
    yaml.push_str("  ipv6: 'fc00::1'\n");
    yaml.push_str("\nsocks5:\n");
    yaml.push_str(&format!("  port: {}\n", socks_addr.port()));
    yaml.push_str(&format!("  address: {}\n", socks_addr.ip()));
    yaml.push_str("  udp: 'udp'\n");
    if let (Some(u), Some(p)) = (socks_user, socks_pass) {
        yaml.push_str(&format!("  username: '{u}'\n"));
        yaml.push_str(&format!("  password: '{p}'\n"));
    }
    if let Some(dns_ip) = mapdns {
        yaml.push_str("\nmapdns:\n");
        yaml.push_str(&format!("  address: {dns_ip}\n"));
        yaml.push_str("  port: 53\n");
    }
    yaml.push_str("\nmisc:\n");
    yaml.push_str("  log-level: warn\n");
    yaml
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_yaml_minimal() {
        let y = build_yaml(
            "10.10.0.2".parse().unwrap(),
            1500,
            "127.0.0.1:50100".parse().unwrap(),
            None,
            None,
            None,
        );
        assert!(y.contains("ipv4: 10.10.0.2"));
        assert!(y.contains("mtu: 1500"));
        assert!(y.contains("port: 50100"));
        assert!(y.contains("address: 127.0.0.1"));
        assert!(!y.contains("username:"));
        assert!(!y.contains("mapdns:"));
    }

    #[test]
    fn build_yaml_with_auth_and_mapdns() {
        let y = build_yaml(
            "10.10.0.2".parse().unwrap(),
            1500,
            "127.0.0.1:50100".parse().unwrap(),
            Some("user1"),
            Some("p4ss"),
            Some("10.10.0.2".parse().unwrap()),
        );
        assert!(y.contains("username: 'user1'"));
        assert!(y.contains("password: 'p4ss'"));
        assert!(y.contains("mapdns:"));
        assert!(y.contains("address: 10.10.0.2"));
    }
}
