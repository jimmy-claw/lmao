//! Native waku-bindings (libwaku FFI) transport.
//!
//! Wraps the `waku-bindings` crate to provide a native Waku relay transport
//! without needing a separate nwaku REST node. Requires libwaku (Nim) at build time.
//!
//! Enable with `cargo build --features native-waku`.
//!
//! See: <https://github.com/logos-messaging/logos-delivery-rust-bindings>

use crate::Transport;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use waku_bindings::{
    waku_new, Encoding, LibwakuResponse, WakuContentTopic, WakuEvent, WakuMessage,
    WakuNodeConfig, WakuNodeHandle, Running,
};
use waku_bindings::general::pubsubtopic::PubsubTopic;

const DEFAULT_PUBSUB_TOPIC: &str = "/waku/2/default-waku/proto";

/// Assert that a future is `Send`.
///
/// # Safety
/// The caller must ensure the future is only awaited on one thread at a time.
/// This is safe for libwaku FFI futures because the underlying C library
/// serialises all operations through an internal event loop.
unsafe fn assert_send<T>(fut: impl Future<Output = T>) -> impl Future<Output = T> + Send {
    struct AssertSend<F>(F);
    unsafe impl<F> Send for AssertSend<F> {}
    impl<F: Future> Future for AssertSend<F> {
        type Output = F::Output;
        fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
            // SAFETY: pinning is structural
            unsafe { self.map_unchecked_mut(|s| &mut s.0).poll(cx) }
        }
    }
    AssertSend(fut)
}

/// Native Waku transport using libwaku FFI via waku-bindings.
pub struct NativeWakuTransport {
    node: WakuNodeHandle<Running>,
    /// Topic → sender for dispatching incoming messages to subscribers.
    senders: Arc<Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>>,
}

// SAFETY: `WakuNodeHandle<Running>` contains a `*mut c_void` (nwaku context)
// which prevents auto-deriving Send/Sync. The underlying libwaku C library
// serialises all FFI calls through an internal event loop, making cross-thread
// usage of the context pointer safe.
unsafe impl Send for NativeWakuTransport {}
unsafe impl Sync for NativeWakuTransport {}

impl NativeWakuTransport {
    /// Create and start a new Waku node with the given config.
    pub async fn new(config: WakuNodeConfig) -> Result<Self> {
        let node = waku_new(Some(config))
            .await
            .map_err(|e| anyhow::anyhow!("waku_new failed: {}", e))?;

        let senders: Arc<Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let senders_clone = senders.clone();
        node.set_event_callback(move |response: LibwakuResponse| {
            if let LibwakuResponse::Success(Some(json)) = response {
                if let Ok(event) = serde_json::from_str::<WakuEvent>(&json) {
                    if let WakuEvent::WakuMessage(msg_event) = event {
                        let content_topic = msg_event.waku_message.content_topic.to_string();
                        let payload = msg_event.waku_message.payload.clone();
                        if let Ok(senders) = senders_clone.lock() {
                            if let Some(tx) = senders.get(&content_topic) {
                                let _ = tx.try_send(payload);
                            }
                        }
                    }
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("set_event_callback failed: {}", e))?;

        // SAFETY: waku FFI futures are safe to send across threads
        let node = unsafe { assert_send(node.start()) }
            .await
            .map_err(|e| anyhow::anyhow!("waku start failed: {}", e))?;

        unsafe {
            assert_send(node.relay_subscribe(&PubsubTopic::new(DEFAULT_PUBSUB_TOPIC)))
        }
            .await
            .map_err(|e| anyhow::anyhow!("relay_subscribe failed: {}", e))?;

        Ok(Self { node, senders })
    }

    /// Connect to a peer by multiaddress.
    pub async fn connect(&self, addr: &str) -> Result<()> {
        let multiaddr: waku_bindings::Multiaddr = addr
            .parse()
            .context("invalid multiaddress")?;
        // SAFETY: waku FFI futures are safe to send across threads
        unsafe { assert_send(self.node.connect(&multiaddr, None)) }
            .await
            .map_err(|e| anyhow::anyhow!("connect failed: {}", e))
    }

    /// Get the listening multiaddresses of this node.
    pub async fn listen_addresses(&self) -> Result<Vec<String>> {
        // SAFETY: waku FFI futures are safe to send across threads
        let addrs = unsafe { assert_send(self.node.listen_addresses()) }
            .await
            .map_err(|e| anyhow::anyhow!("listen_addresses failed: {}", e))?;
        Ok(addrs.into_iter().map(|a| a.to_string()).collect())
    }
}

/// Map an application-level topic string to a WakuContentTopic.
fn make_content_topic(topic: &str) -> WakuContentTopic {
    WakuContentTopic {
        application_name: std::borrow::Cow::Borrowed("a2a"),
        version: std::borrow::Cow::Borrowed("1"),
        content_topic_name: std::borrow::Cow::Owned(topic.to_string()),
        encoding: Encoding::Proto,
    }
}

#[async_trait]
impl Transport for NativeWakuTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let content_topic = make_content_topic(topic);
        let message = WakuMessage::new(payload, content_topic, 0, Vec::<u8>::new(), false);
        let pubsub_topic = PubsubTopic::new(DEFAULT_PUBSUB_TOPIC);

        // SAFETY: waku FFI futures are safe to send across threads
        unsafe {
            assert_send(self.node.relay_publish_message(&message, &pubsub_topic, None))
        }
            .await
            .map_err(|e| anyhow::anyhow!("publish failed: {}", e))?;
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let content_topic = make_content_topic(topic);
        let content_topic_str = content_topic.to_string();

        let (tx, rx) = mpsc::channel(256);

        self.senders
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?
            .insert(content_topic_str, tx);

        Ok(rx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        let content_topic = make_content_topic(topic);
        let content_topic_str = content_topic.to_string();

        self.senders
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?
            .remove(&content_topic_str);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_content_topic() {
        let topic = make_content_topic("my-agent");
        let s = topic.to_string();
        assert!(s.contains("a2a"));
        assert!(s.contains("my-agent"));
    }
}
