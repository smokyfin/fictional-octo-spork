//! Pluggable Transport (PT) abstraction.
//!
//! The PT layer presents Arti with a SOCKS5 proxy that anonymises Arti's
//! outbound TCP connections. In our pipeline the SOCKS5 proxy is implemented
//! by Leaf #1, but the abstraction allows us to swap in `lyrebird`,
//! `xray-core`, or any future provider without changing the orchestrator.

pub mod leaf;

use crate::error::Result;
use crate::runtime::{Cancel, SocksEndpoint};
use async_trait::async_trait;

/// One running pluggable-transport endpoint that Arti can route bridge traffic
/// through. Implementations own the supporting tasks; dropping the value
/// must release them, and `shutdown()` must do so promptly.
#[async_trait]
pub trait PluggableTransport: Send + Sync {
    /// SOCKS5 endpoint (with auth) Arti should use as its bridge transport.
    fn socks(&self) -> &SocksEndpoint;

    /// Symbolic name of the provider (used by logs / dev tab).
    fn name(&self) -> &'static str;

    /// Stop the transport and release its resources.
    async fn shutdown(&self);
}

/// Provider selector — chosen by config / UI. Currently only Leaf is wired,
/// but the type is future-proof for Lyrebird / Xray providers.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Provider {
    #[default]
    Leaf,
    Lyrebird, // not yet implemented; reserved for future use.
    Xray,     // not yet implemented; reserved for future use.
}

/// Spawn the configured pluggable transport. The returned trait object owns
/// the underlying provider's runtime tasks and must be `shutdown()`-ed when
/// the engine stops.
pub async fn spawn(provider: Provider, cancel: Cancel) -> Result<Box<dyn PluggableTransport>> {
    match provider {
        Provider::Leaf => leaf::LeafSocksPt::spawn(cancel)
            .await
            .map(|pt| Box::new(pt) as Box<dyn PluggableTransport>),
        Provider::Lyrebird | Provider::Xray => Err(crate::Error::Other(format!(
            "pluggable transport {provider:?} is not implemented yet"
        ))),
    }
}
