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
        let result =
            logos_core::call_plugin_method(PLUGIN, "createNode", &params_json(&[("cfg", cfg)]))
                .await
                .map_err(|e| anyhow::anyhow!("delivery_module createNode failed: {}", e))?;
        if result != "true" {
            bail!("delivery_module createNode returned: {}", result);
        }

        // start
        let result = logos_core::call_plugin_method(PLUGIN, "start", "[]")
            .await
            .map_err(|e| anyhow::anyhow!("delivery_module start failed: {}", e))?;
        if result != "true" {
            bail!("delivery_module start returned: {}", result);
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

                    let payload =
                        match base64::engine::general_purpose::STANDARD.decode(payload_b64) {
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
        .await
        .map_err(|e| anyhow::anyhow!("delivery_module send failed: {}", e))?;

        if result.starts_with("error") || result.starts_with("Error") {
            bail!("delivery_module send returned error: {}", result);
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let result = logos_core::call_plugin_method(
            PLUGIN,
            "subscribe",
            &params_json(&[("contentTopic", topic)]),
        )
        .await
        .map_err(|e| anyhow::anyhow!("delivery_module subscribe failed: {}", e))?;
        if result != "true" {
            bail!("delivery_module subscribe returned: {}", result);
        }

        let (tx, rx_unbounded) = mpsc::unbounded_channel();
        self.subscriptions
            .lock()
            .unwrap()
            .insert(topic.to_string(), tx);

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
        self.subscriptions.lock().unwrap().remove(topic);

        let result = logos_core::call_plugin_method(
            PLUGIN,
            "unsubscribe",
            &params_json(&[("contentTopic", topic)]),
        )
        .await
        .map_err(|e| anyhow::anyhow!("delivery_module unsubscribe failed: {}", e))?;
        if result != "true" {
            bail!("delivery_module unsubscribe returned: {}", result);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_json_empty() {
        assert_eq!(params_json(&[]), "[]");
    }

    #[test]
    fn params_json_single_pair() {
        let result = params_json(&[("cfg", "value1")]);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "cfg");
        assert_eq!(arr[0]["value"], "value1");
        assert_eq!(arr[0]["type"], "string");
    }

    #[test]
    fn params_json_multiple_pairs() {
        let result = params_json(&[("contentTopic", "/my/topic"), ("payload", "abc123")]);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "contentTopic");
        assert_eq!(arr[0]["value"], "/my/topic");
        assert_eq!(arr[1]["name"], "payload");
        assert_eq!(arr[1]["value"], "abc123");
    }

    #[test]
    fn params_json_is_valid_json() {
        let result = params_json(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&result);
        assert!(parsed.is_ok(), "params_json should produce valid JSON");
    }

    #[test]
    fn plugin_constant() {
        assert_eq!(PLUGIN, "delivery_module");
    }
}
