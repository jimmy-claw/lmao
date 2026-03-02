use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

pub mod memory;
#[cfg(feature = "rest")]
pub mod nwaku_rest;
pub mod sds;

#[cfg(feature = "logos-core")]
mod logos_core;
#[cfg(feature = "logos-core")]
pub mod logos_core_transport;
#[cfg(feature = "logos-core")]
pub use logos_core_transport::LogosCoreDeliveryTransport;

#[cfg(feature = "native-waku")]
mod waku_bindings_transport;
#[cfg(feature = "native-waku")]
pub use waku_bindings_transport::NativeWakuTransport;

/// Swappable transport trait — real nwaku in production, in-memory mock in tests.
///
/// Implementations:
/// - `LogosMessagingTransport`: nwaku REST API (requires running nwaku node, `rest` feature)
/// - `LogosCoreDeliveryTransport`: Logos Core IPC via delivery_module (`logos-core` feature)
/// - `NativeWakuTransport`: native libwaku FFI via waku-bindings (`native-waku` feature)
/// - `InMemoryTransport`: in-process mock for testing (no external deps)
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Publish a payload to a content topic.
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()>;

    /// Subscribe to a content topic. Returns a channel receiver for incoming messages.
    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>>;

    /// Unsubscribe from a content topic.
    async fn unsubscribe(&self, topic: &str) -> Result<()>;
}
