//! Leaf-backed Pluggable Transport for Arti.
//!
//! Spawns Leaf with a SOCKS5 inbound (random port + random user/password) and
//! a default outbound (`direct` — Arti's bridge connection is what gets
//! tunnelled). Arti then uses this SOCKS5 proxy as the transport for its
//! bridge connection.
//!
//! The "Leaf-as-PT" abstraction means this file is the *only* place that
//! talks to Leaf for the PT role; switching to Lyrebird/Xray would mean
//! adding a sibling module.

use super::PluggableTransport;
use crate::error::{Error, Result};
use crate::runtime::{Cancel, SocksEndpoint};
use async_trait::async_trait;
use tokio::task::JoinHandle;
use tracing::info;

pub struct LeafSocksPt {
    socks: SocksEndpoint,
    cancel: Cancel,
    task: parking_lot::Mutex<Option<JoinHandle<()>>>,
}

impl LeafSocksPt {
    pub async fn spawn(cancel: Cancel) -> Result<Self> {
        let socks = SocksEndpoint::new_random()?;
        info!(addr = %socks.addr, "starting Leaf-PT SOCKS5 inbound");

        let cfg = crate::engine::leaf_config::pt_socks_inbound_config(&socks)?;
        let task =
            crate::engine::leaf_config::run_leaf_with_config("leaf-pt", cfg, cancel.clone())?;

        Ok(Self {
            socks,
            cancel,
            task: parking_lot::Mutex::new(Some(task)),
        })
    }
}

#[async_trait]
impl PluggableTransport for LeafSocksPt {
    fn socks(&self) -> &SocksEndpoint {
        &self.socks
    }

    fn name(&self) -> &'static str {
        "leaf"
    }

    async fn shutdown(&self) {
        self.cancel.cancel();
        // Take the JoinHandle out of the mutex inside a small scope so the
        // (non-Send) parking_lot guard is dropped *before* we hit `.await`.
        let handle = { self.task.lock().take() };
        if let Some(handle) = handle {
            // leaf::shutdown() inside the task already breaks its event loop;
            // abort is just a belt-and-braces safety net.
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for LeafSocksPt {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.task.lock().take() {
            handle.abort();
        }
    }
}

#[allow(dead_code)]
fn _err(s: impl Into<String>) -> Error {
    Error::Leaf(s.into())
}
