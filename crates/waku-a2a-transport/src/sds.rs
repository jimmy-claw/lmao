//! Minimal SDS (Scalable Data Sync) — reliable delivery layer.
//!
//! Waku is fire-and-forget. For agent tasks we need delivery guarantees.
//! This implements a minimal SDS-inspired protocol:
//!
//! - Each message has a UUID
//! - Sender publishes and then waits for ACK on /waku-a2a/1/ack/{message_id}/proto
//! - Receiver sends ACK after processing
//! - If no ACK within timeout: retransmit up to MAX_RETRIES times
//!
//! TODO (Issue #2): Replace with the full SDS protocol spec.
//! Reference: https://blog.waku.org/explanation-series-a-unified-stack-for-scalable-and-reliable-p2p-communication/

use crate::Transport;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

const ACK_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RETRIES: u32 = 3;

/// Minimal SDS layer wrapping any Transport.
pub struct SdsTransport<T: Transport> {
    inner: T,
    /// Bloom filter substitute: set of seen message IDs for deduplication.
    /// TODO (Issue #2): Replace with proper bloom filter from SDS spec.
    seen_ids: Mutex<HashSet<String>>,
}

impl<T: Transport> SdsTransport<T> {
    pub fn new(transport: T) -> Self {
        Self {
            inner: transport,
            seen_ids: Mutex::new(HashSet::new()),
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Publish with SDS reliability: retransmit up to MAX_RETRIES times
    /// if no ACK is received within ACK_TIMEOUT.
    pub async fn publish_reliable(
        &self,
        topic: &str,
        payload: &[u8],
        message_id: &str,
    ) -> Result<bool> {
        let ack_topic = format!("/waku-a2a/1/ack/{}/proto", message_id);
        let mut ack_rx = self.inner.subscribe(&ack_topic).await?;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tracing_log(&format!(
                    "SDS: retransmit attempt {}/{} for {}",
                    attempt, MAX_RETRIES, message_id
                ));
            }

            self.inner
                .publish(topic, payload)
                .await
                .context("SDS publish failed")?;

            // Wait for ACK with timeout
            match tokio::time::timeout(ACK_TIMEOUT, wait_for_ack(&mut ack_rx, message_id)).await {
                Ok(true) => {
                    let _ = self.inner.unsubscribe(&ack_topic).await;
                    return Ok(true);
                }
                _ => continue,
            }
        }

        let _ = self.inner.unsubscribe(&ack_topic).await;
        tracing_log(&format!(
            "SDS: no ACK after {} retries for {}",
            MAX_RETRIES, message_id
        ));
        Ok(false)
    }

    /// Send an ACK for a received message.
    pub async fn send_ack(&self, message_id: &str) -> Result<()> {
        let ack_topic = format!("/waku-a2a/1/ack/{}/proto", message_id);
        let ack_payload = serde_json::to_vec(&serde_json::json!({
            "type": "ack",
            "message_id": message_id,
        }))?;
        self.inner.publish(&ack_topic, &ack_payload).await
    }

    /// Check if a message ID has been seen before (deduplication).
    pub fn is_duplicate(&self, message_id: &str) -> bool {
        let seen = self.seen_ids.lock().unwrap();
        seen.contains(message_id)
    }

    /// Mark a message ID as seen.
    pub fn mark_seen(&self, message_id: &str) {
        let mut seen = self.seen_ids.lock().unwrap();
        seen.insert(message_id.to_string());
    }

    /// Filter duplicate messages from a batch, using the "id" field in JSON.
    pub fn filter_dedup(&self, messages: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        messages
            .into_iter()
            .filter(|msg| {
                if let Ok(envelope) = serde_json::from_slice::<serde_json::Value>(msg) {
                    if let Some(id) = envelope.get("id").and_then(|v| v.as_str()) {
                        if self.is_duplicate(id) {
                            return false;
                        }
                        self.mark_seen(id);
                    }
                }
                true
            })
            .collect()
    }
}

/// Wait for an ACK message on a channel.
async fn wait_for_ack(rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>, message_id: &str) -> bool {
    while let Some(msg) = rx.recv().await {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&msg) {
            if val.get("message_id").and_then(|v| v.as_str()) == Some(message_id) {
                return true;
            }
        }
    }
    false
}

fn tracing_log(msg: &str) {
    eprintln!("[sds] {}", msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryTransport;

    #[test]
    fn test_deduplication() {
        let transport = InMemoryTransport::new();
        let sds = SdsTransport::new(transport);

        assert!(!sds.is_duplicate("msg-1"));
        sds.mark_seen("msg-1");
        assert!(sds.is_duplicate("msg-1"));
        assert!(!sds.is_duplicate("msg-2"));
    }

    #[tokio::test]
    async fn test_send_ack() {
        let transport = InMemoryTransport::new();
        let spy = transport.clone();
        let sds = SdsTransport::new(transport);

        sds.send_ack("task-123").await.unwrap();

        // Verify ACK was published to correct topic
        let mut rx = spy
            .subscribe("/waku-a2a/1/ack/task-123/proto")
            .await
            .unwrap();
        let msg = rx.try_recv().unwrap();
        let val: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        assert_eq!(val["message_id"], "task-123");
    }

    #[tokio::test]
    async fn test_publish_reliable_with_ack() {
        let transport = InMemoryTransport::new();

        // Pre-publish an ACK (will be replayed via history on subscribe)
        let ack_payload = serde_json::to_vec(&serde_json::json!({
            "type": "ack",
            "message_id": "msg-1",
        }))
        .unwrap();
        transport
            .publish("/waku-a2a/1/ack/msg-1/proto", &ack_payload)
            .await
            .unwrap();

        let sds = SdsTransport::new(transport);

        let acked = sds
            .publish_reliable("/waku-a2a/1/task/somepubkey/proto", b"hello", "msg-1")
            .await
            .unwrap();
        assert!(acked);
    }

    #[test]
    fn test_filter_dedup() {
        let transport = InMemoryTransport::new();
        let sds = SdsTransport::new(transport);

        let msg = serde_json::to_vec(&serde_json::json!({
            "id": "task-1",
            "data": "hello"
        }))
        .unwrap();

        // Same message twice — second should be deduped
        let result = sds.filter_dedup(vec![msg.clone(), msg]);
        assert_eq!(result.len(), 1);
    }
}
