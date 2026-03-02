//! Native waku-bindings FFI transport (Issue #18).
//!
//! Wraps `waku_new` / `relay_subscribe` / `relay_publish_message` from
//! `waku-bindings` to provide an embedded libwaku transport — no external
//! nwaku REST node required.
//!
//! Requires the `native` Cargo feature and a Nim-built `libwaku` on the
//! link path.  See the repo README for build instructions.

use crate::Transport;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use waku_bindings::{
    Encoding, LibwakuResponse, WakuContentTopic, WakuEvent, WakuMessage, WakuNodeConfig,
    WakuNodeHandle, Running,
};
use waku_bindings::node::PubsubTopic;

const DEFAULT_PUBSUB_TOPIC: &str = "/waku/2/default-waku/proto";

/// Native libwaku FFI transport.
///
/// Embeds a full Waku relay node in-process via `waku-bindings`.
/// Messages are received through the event callback and routed to
/// per-content-topic mpsc channels.
pub struct NativeTransport {
    node: WakuNodeHandle<Running>,
    pubsub_topic: PubsubTopic,
    /// Senders keyed by content-topic string.  The event callback pushes
    /// incoming messages into the matching sender.
    senders: Arc<Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>>,
}

impl NativeTransport {
    /// Create and start a native Waku node with default config.
    pub async fn new(config: Option<WakuNodeConfig>) -> Result<Self> {
        let senders: Arc<Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let node = waku_bindings::waku_new(config)
            .await
            .map_err(|e| anyhow::anyhow!("waku_new failed: {}", e))?;

        // Wire the event callback so incoming relay messages are routed to
        // the correct per-topic mpsc sender.
        let senders_cb = Arc::clone(&senders);
        node.set_event_callback(move |response| {
            if let LibwakuResponse::Success(Some(json)) = response {
                if let Ok(WakuEvent::WakuMessage(evt)) = serde_json::from_str(&json) {
                    let topic = evt.waku_message.content_topic.to_string();
                    let payload = evt.waku_message.payload.clone();
                    if let Ok(guard) = senders_cb.lock() {
                        if let Some(tx) = guard.get(&topic) {
                            // Best-effort send — drop if the receiver is full/gone.
                            let _ = tx.try_send(payload);
                        }
                    }
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("set_event_callback failed: {}", e))?;

        let node = node
            .start()
            .await
            .map_err(|e| anyhow::anyhow!("waku start failed: {}", e))?;

        let pubsub_topic = PubsubTopic::new(DEFAULT_PUBSUB_TOPIC);

        // Subscribe to the default pubsub topic so the relay node joins
        // the mesh.
        node.relay_subscribe(&pubsub_topic)
            .await
            .map_err(|e| anyhow::anyhow!("relay_subscribe failed: {}", e))?;

        Ok(Self {
            node,
            pubsub_topic,
            senders,
        })
    }
}

#[async_trait]
impl Transport for NativeTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let content_topic: WakuContentTopic = topic
            .parse()
            .map_err(|e: String| anyhow::anyhow!("bad content topic: {}", e))?;

        let message = WakuMessage::new(payload, content_topic, 0, Vec::<u8>::new(), false);

        self.node
            .relay_publish_message(&message, &self.pubsub_topic, None)
            .await
            .map_err(|e| anyhow::anyhow!("relay_publish_message: {}", e))?;

        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let (tx, rx) = mpsc::channel(256);
        self.senders
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?
            .insert(topic.to_string(), tx);
        Ok(rx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        self.senders
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?
            .remove(topic);
        Ok(())
    }
}
