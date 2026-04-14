//! QtRO-based delivery transport — inter-module calls via `logos_core_call_plugin_method_async`.
//!
//! Calls the real `logos-delivery-module` (logos-co/logos-delivery-module) through the
//! Logos Core C IPC layer. Unlike `LogosCoreDeliveryTransport`, this transport does NOT
//! manage the delivery node lifecycle (`createNode`/`start`) — the external
//! `delivery_module` plugin manages its own lifecycle.
//!
//! Replaces the previous callback-based approach (PublishFn/SubscribeFn/UnsubscribeFn)
//! with direct `logos_core_call_plugin_method_async` calls and
//! `logos_core_register_event_listener` for inbound messages.

use crate::{Result, Transport, TransportError};
use async_trait::async_trait;
use base64::Engine;
use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

// Re-use the logos_core FFI bindings (call_plugin_method, register_event_listener).
use crate::logos_core;

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

// ---------------------------------------------------------------------------
// Inbound message dispatch (C++ → Rust) — kept for backward compatibility
// with C++ hosts that call lmao_qtro_on_message directly.
// ---------------------------------------------------------------------------

/// Per-topic sender map for dispatching inbound messages.
static TOPIC_SENDERS: OnceLock<Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>>> =
    OnceLock::new();

fn topic_senders() -> &'static Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>> {
    TOPIC_SENDERS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Called from C++ when the delivery_module emits a message on a subscribed topic.
///
/// Retained for backward compatibility with existing C++ module hosts.
///
/// # Safety
/// `topic` and `payload_b64` must be valid, null-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn lmao_qtro_on_message(topic: *const c_char, payload_b64: *const c_char) {
    let topic_str = match unsafe { CStr::from_ptr(topic) }.to_str() {
        Ok(s) => s,
        Err(_) => return,
    };
    let payload_str = match unsafe { CStr::from_ptr(payload_b64) }.to_str() {
        Ok(s) => s,
        Err(_) => return,
    };

    let payload = match base64::engine::general_purpose::STANDARD.decode(payload_str) {
        Ok(p) => p,
        Err(_) => return,
    };

    let guard = topic_senders().lock().unwrap();
    if let Some(tx) = guard.get(topic_str) {
        let _ = tx.send(payload);
    }
}

// ---------------------------------------------------------------------------
// Transport implementation
// ---------------------------------------------------------------------------

/// Logos Delivery transport via QtRO inter-module calls.
///
/// Uses `logos_core_call_plugin_method_async` to call the `delivery_module`
/// plugin directly through Logos Core's C IPC layer. Inbound messages arrive
/// via `logos_core_register_event_listener("delivery_module", "messageReceived")`.
///
/// The delivery_module manages its own node lifecycle — this transport only
/// calls `send`, `subscribe`, and `unsubscribe`.
pub struct QtRODeliveryTransport {
    subscriptions: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>>,
    /// Keep the event listener state alive so the FFI callback pointer stays valid.
    _event_listener_state: Box<logos_core::EventListenerState>,
}

impl QtRODeliveryTransport {
    /// Create a new QtRO delivery transport.
    ///
    /// Registers a `messageReceived` event listener on the `delivery_module`
    /// plugin to receive inbound messages. Does NOT call `createNode` or
    /// `start` — the external delivery_module manages its own lifecycle.
    pub fn new() -> Result<Self> {
        let subscriptions: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Also register in the global topic_senders for backward compat with
        // C++ hosts that call lmao_qtro_on_message directly.
        let global_subs = Arc::clone(&subscriptions);

        // Register event listener for inbound messages from delivery_module.
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

        // Sync the global topic_senders so lmao_qtro_on_message also works.
        let global_ref = Arc::clone(&subscriptions);
        let _ = TOPIC_SENDERS.get_or_init(|| global_ref);

        Ok(Self {
            subscriptions,
            _event_listener_state: event_state,
        })
    }
}

#[async_trait]
impl Transport for QtRODeliveryTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let result = logos_core::call_plugin_method(
            PLUGIN,
            "send",
            &params_json(&[("contentTopic", topic), ("payload", &payload_b64)]),
        )
        .await
        .map_err(|e| TransportError::Transport(format!("delivery_module send failed: {}", e)))?;

        if result.starts_with("error") || result.starts_with("Error") {
            return Err(TransportError::Transport(format!(
                "delivery_module send returned error: {}",
                result
            )));
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
        .map_err(|e| {
            TransportError::Transport(format!("delivery_module subscribe failed: {}", e))
        })?;
        if result != "true" {
            return Err(TransportError::Transport(format!(
                "delivery_module subscribe returned: {}",
                result
            )));
        }

        let (tx, rx_unbounded) = mpsc::unbounded_channel();
        self.subscriptions
            .lock()
            .unwrap()
            .insert(topic.to_string(), tx);

        // Bridge unbounded → bounded channel (backpressure).
        let (btx, brx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut rx = rx_unbounded;
            while let Some(msg) = rx.recv().await {
                if btx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        Ok(brx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        // Remove local sender first.
        self.subscriptions.lock().unwrap().remove(topic);

        let result = logos_core::call_plugin_method(
            PLUGIN,
            "unsubscribe",
            &params_json(&[("contentTopic", topic)]),
        )
        .await
        .map_err(|e| {
            TransportError::Transport(format!("delivery_module unsubscribe failed: {}", e))
        })?;
        if result != "true" {
            return Err(TransportError::Transport(format!(
                "delivery_module unsubscribe returned: {}",
                result
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

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
        let parsed: std::result::Result<serde_json::Value, _> = serde_json::from_str(&result);
        assert!(parsed.is_ok(), "params_json should produce valid JSON");
    }

    #[test]
    fn plugin_constant() {
        assert_eq!(PLUGIN, "delivery_module");
    }

    #[test]
    fn topic_senders_dispatch() {
        let senders = topic_senders();
        let (tx, mut rx) = mpsc::unbounded_channel();
        senders.lock().unwrap().insert("test/topic".into(), tx);

        let topic = CString::new("test/topic").unwrap();
        let payload = base64::engine::general_purpose::STANDARD.encode(b"hello");
        let payload_c = CString::new(payload).unwrap();

        unsafe {
            lmao_qtro_on_message(topic.as_ptr(), payload_c.as_ptr());
        }

        let received = rx.try_recv().unwrap();
        assert_eq!(received, b"hello");

        // Clean up
        senders.lock().unwrap().remove("test/topic");
    }

    #[test]
    fn on_message_unknown_topic_ignored() {
        let topic = CString::new("no/such/topic").unwrap();
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(b"data");
        let payload = CString::new(payload_b64).unwrap();

        // Should not panic
        unsafe {
            lmao_qtro_on_message(topic.as_ptr(), payload.as_ptr());
        }
    }

    #[test]
    fn on_message_invalid_base64_ignored() {
        let senders = topic_senders();
        let (tx, mut rx) = mpsc::unbounded_channel();
        senders.lock().unwrap().insert("b64test".into(), tx);

        let topic = CString::new("b64test").unwrap();
        let payload = CString::new("not-valid-base64!").unwrap();

        unsafe {
            lmao_qtro_on_message(topic.as_ptr(), payload.as_ptr());
        }

        // Should not have dispatched anything
        assert!(rx.try_recv().is_err());

        senders.lock().unwrap().remove("b64test");
    }
}
