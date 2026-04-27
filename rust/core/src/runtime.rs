//! Shared runtime primitives: random SOCKS endpoints + cancellation.

use crate::util::{random_local_port, random_password, random_username};
use std::net::SocketAddr;
use tokio::sync::watch;

/// A randomised SOCKS5 endpoint with username/password auth, used to wire
/// up the internal pipeline (Leaf → Arti → Leaf). Bound to `127.0.0.1`
/// inside the VPN process; not reachable from outside the device.
#[derive(Debug, Clone)]
pub struct SocksEndpoint {
    pub addr: SocketAddr,
    pub username: String,
    pub password: String,
}

impl SocksEndpoint {
    pub fn new_random() -> std::io::Result<Self> {
        let port = random_local_port()?;
        Ok(Self {
            addr: ([127, 0, 0, 1], port).into(),
            username: random_username(12),
            password: random_password(32),
        })
    }
}

/// Lightweight cancellation token. Cloning gives child tasks a handle that
/// resolves the moment `cancel()` is called on any clone.
#[derive(Debug, Clone)]
pub struct Cancel {
    rx: watch::Receiver<bool>,
    tx: watch::Sender<bool>,
}

impl Cancel {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx, rx }
    }

    pub fn cancel(&self) {
        let _ = self.tx.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Future that resolves the moment the token is cancelled.
    pub async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        if *rx.borrow() {
            return;
        }
        // `changed()` errors only if the sender was dropped, which we treat
        // as "cancelled" anyway.
        let _ = rx.changed().await;
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}
