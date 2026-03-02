use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

pub mod memory;
pub mod nwaku_rest;
pub mod sds;

#[cfg(feature = "native")]
pub mod logos_messaging;

/// Swappable transport trait — real nwaku in production, in-memory mock in tests.
///
/// Implementations:
/// - `LogosMessagingTransport`: nwaku REST API (requires running nwaku node)
/// - `InMemoryTransport`: in-process mock for testing (no external deps)
/// - `NativeTransport`: embedded libwaku FFI via waku-bindings (feature = "native")
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Publish a payload to a content topic.
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()>;

    /// Subscribe to a content topic. Returns a channel receiver for incoming messages.
    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>>;

    /// Unsubscribe from a content topic.
    async fn unsubscribe(&self, topic: &str) -> Result<()>;
}
