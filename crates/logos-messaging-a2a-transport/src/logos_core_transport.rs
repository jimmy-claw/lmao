//! Logos Core delivery-module transport — native IPC via `logos_core_call_plugin_method_async`.
//!
//! Communicates with the `delivery_module` plugin through Logos Core's C IPC layer
//! instead of the nwaku REST API. Requires the `logos-core` feature and linking
//! against `liblogos_core`.

use crate::logos_core;
use crate::Transport;
use anyhow::{bail, Result};
use async_trait::async_trait;
use base64::Engine;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const PLUGIN: &str = "delivery_module";

/// Build a Logos Core params JSON array from name/value pairs (all string-typed).
fn params_json(pairs: &[(&str, &str)]) -> String {
    let entries: Vec<String> = pairs
        .iter()
        .map(|(name, value)| {
            format!(
                r#"{{"name":"{}","value":"{}","type":"string"}}"#,
                name, value
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

/// Transport implementation backed by Logos Core IPC to the `delivery_module` plugin.
///
/// Uses `logos_core_call_plugin_method_async` for publish/subscribe/unsubscribe and
/// `logos_core_register_event_listener` for receiving messages via the `messageReceived` event.
pub struct LogosCoreDeliveryTransport {
    subscriptions: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>>,
    /// Keep the event listener state alive so the FFI callback pointer stays valid.
    _event_listener_state: Box<logos_core::EventListenerState>,
}

impl LogosCoreDeliveryTransport {
    /// Create and start a new delivery-module transport.
    ///
    /// Calls `createNode(cfg)` then `start()` on the delivery_module plugin, and
    /// registers a global `messageReceived` event listener that fans out to per-topic channels.
    pub async fn new(cfg: &str) -> Result<Self> {
        // createNode
        let result = logos_core::call_plugin_method(
            PLUGIN,
            "createNode",
            &params_json(&[("cfg", cfg)]),
        )
        .await;
        if result != "true" {
            bail!("delivery_module createNode failed: {}", result);
        }

        // start
        let result =
            logos_core::call_plugin_method(PLUGIN, "start", "[]").await;
        if result != "true" {
            bail!("delivery_module start failed: {}", result);
        }

        let subscriptions: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Register global messageReceived event listener
        let (mut event_rx, event_state) =
            logos_core::register_event_listener(PLUGIN, "messageReceived");

        let subs = Arc::clone(&subscriptions);
        tokio::spawn(async move {
            while let Some(event_json) = event_rx.recv().await {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&event_json) {
                    let topic = val
                        .get("contentTopic")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let payload_b64 = val
                        .get("payload")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();

                    let payload = match base64::engine::general_purpose::STANDARD
                        .decode(payload_b64)
                    {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    let guard = subs.lock().unwrap();
                    if let Some(tx) = guard.get(topic) {
                        let _ = tx.send(payload);
                    }
                }
            }
        });

        Ok(Self {
            subscriptions,
            _event_listener_state: event_state,
        })
    }
}

#[async_trait]
impl Transport for LogosCoreDeliveryTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let result = logos_core::call_plugin_method(
            PLUGIN,
            "send",
            &params_json(&[("contentTopic", topic), ("payload", &payload_b64)]),
        )
        .await;

        // send() returns a QExpected<QString>; a non-error result is success
        if result.starts_with("error") || result.starts_with("Error") {
            bail!("delivery_module send failed: {}", result);
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        // Tell the delivery module to subscribe to this content topic
        let result = logos_core::call_plugin_method(
            PLUGIN,
            "subscribe",
            &params_json(&[("contentTopic", topic)]),
        )
        .await;
        if result != "true" {
            bail!("delivery_module subscribe failed: {}", result);
        }

        // Create a channel for this topic and register it for event fan-out
        let (tx, rx_unbounded) = mpsc::unbounded_channel();
        self.subscriptions
            .lock()
            .unwrap()
            .insert(topic.to_string(), tx);

        // Bridge unbounded → bounded channel (matching Transport trait signature)
        let (btx, brx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut rx_unbounded = rx_unbounded;
            while let Some(msg) = rx_unbounded.recv().await {
                if btx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        Ok(brx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        // Remove the channel first so no more events are routed
        self.subscriptions.lock().unwrap().remove(topic);

        let result = logos_core::call_plugin_method(
            PLUGIN,
            "unsubscribe",
            &params_json(&[("contentTopic", topic)]),
        )
        .await;
        if result != "true" {
            bail!("delivery_module unsubscribe failed: {}", result);
        }
        Ok(())
    }
}
