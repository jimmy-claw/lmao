//! In-memory transport for testing — no nwaku required.
//!
//! Messages published to a topic are broadcast to all subscribers and stored in history.
//! New subscribers receive all historical messages (replay), making it suitable for
//! tests where publish may happen before subscribe.

use crate::Transport;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// In-memory transport backed by shared state.
/// Clone to share between multiple nodes for in-process testing.
#[derive(Clone)]
pub struct InMemoryTransport {
    inner: Arc<Mutex<TransportState>>,
}

struct TransportState {
    subscribers: HashMap<String, Vec<mpsc::Sender<Vec<u8>>>>,
    history: HashMap<String, Vec<Vec<u8>>>,
}

impl InMemoryTransport {
    /// Create a new shared transport. Clone this to give to multiple nodes.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TransportState {
                subscribers: HashMap::new(),
                history: HashMap::new(),
            })),
        }
    }
}

impl Default for InMemoryTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for InMemoryTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let mut state = self.inner.lock().unwrap();
        let data = payload.to_vec();

        // Store in history
        state
            .history
            .entry(topic.to_string())
            .or_default()
            .push(data.clone());

        // Send to all active subscribers (remove dead ones)
        if let Some(subs) = state.subscribers.get_mut(topic) {
            subs.retain(|tx| tx.try_send(data.clone()).is_ok());
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let mut state = self.inner.lock().unwrap();
        let (tx, rx) = mpsc::channel(1024);

        // Replay history to new subscriber
        if let Some(history) = state.history.get(topic) {
            for msg in history {
                let _ = tx.try_send(msg.clone());
            }
        }

        state
            .subscribers
            .entry(topic.to_string())
            .or_default()
            .push(tx);
        Ok(rx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        let mut state = self.inner.lock().unwrap();
        state.subscribers.remove(topic);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_subscribe() {
        let transport = InMemoryTransport::new();
        let mut rx = transport.subscribe("topic-a").await.unwrap();
        transport.publish("topic-a", b"hello").await.unwrap();

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg, b"hello");
    }

    #[tokio::test]
    async fn test_history_replay() {
        let transport = InMemoryTransport::new();
        // Publish BEFORE subscribing
        transport.publish("topic-a", b"msg1").await.unwrap();
        transport.publish("topic-a", b"msg2").await.unwrap();

        // Subscribe gets history
        let mut rx = transport.subscribe("topic-a").await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), b"msg1");
        assert_eq!(rx.recv().await.unwrap(), b"msg2");
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let transport = InMemoryTransport::new();
        let mut rx1 = transport.subscribe("topic-a").await.unwrap();
        let mut rx2 = transport.subscribe("topic-a").await.unwrap();

        transport.publish("topic-a", b"broadcast").await.unwrap();

        assert_eq!(rx1.recv().await.unwrap(), b"broadcast");
        assert_eq!(rx2.recv().await.unwrap(), b"broadcast");
    }

    #[tokio::test]
    async fn test_shared_transport() {
        let t1 = InMemoryTransport::new();
        let t2 = t1.clone(); // Shared state

        let mut rx = t1.subscribe("topic-a").await.unwrap();
        t2.publish("topic-a", b"from t2").await.unwrap();

        assert_eq!(rx.recv().await.unwrap(), b"from t2");
    }

    #[tokio::test]
    async fn test_unsubscribe() {
        let transport = InMemoryTransport::new();
        let _rx = transport.subscribe("topic-a").await.unwrap();
        transport.unsubscribe("topic-a").await.unwrap();

        // Publishing after unsubscribe should not panic
        transport.publish("topic-a", b"hello").await.unwrap();
    }

    #[tokio::test]
    async fn test_topic_isolation() {
        let transport = InMemoryTransport::new();
        let mut rx_a = transport.subscribe("topic-a").await.unwrap();
        let mut rx_b = transport.subscribe("topic-b").await.unwrap();

        transport.publish("topic-a", b"only-a").await.unwrap();

        assert_eq!(rx_a.recv().await.unwrap(), b"only-a");
        // topic-b should have nothing
        assert!(rx_b.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_publish_no_subscribers() {
        let transport = InMemoryTransport::new();
        // Publishing with no subscribers should succeed and store in history
        transport.publish("orphan", b"nobody home").await.unwrap();

        // A later subscriber should still get the message via replay
        let mut rx = transport.subscribe("orphan").await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), b"nobody home");
    }

    #[tokio::test]
    async fn test_dead_subscriber_cleanup() {
        let transport = InMemoryTransport::new();
        let rx = transport.subscribe("topic").await.unwrap();
        // Drop the receiver — this makes the sender "dead"
        drop(rx);

        // Publishing should not panic; dead subscriber is cleaned up
        transport.publish("topic", b"after-drop").await.unwrap();

        // New subscriber should get the full history (both messages)
        let mut rx2 = transport.subscribe("topic").await.unwrap();
        assert_eq!(rx2.recv().await.unwrap(), b"after-drop");
    }

    #[tokio::test]
    async fn test_empty_payload() {
        let transport = InMemoryTransport::new();
        let mut rx = transport.subscribe("topic").await.unwrap();
        transport.publish("topic", b"").await.unwrap();

        let msg = rx.recv().await.unwrap();
        assert!(msg.is_empty());
    }

    #[tokio::test]
    async fn test_history_replay_then_live() {
        let transport = InMemoryTransport::new();
        // Publish before subscribe (becomes history)
        transport.publish("topic", b"history").await.unwrap();

        let mut rx = transport.subscribe("topic").await.unwrap();
        // First message is replayed history
        assert_eq!(rx.recv().await.unwrap(), b"history");

        // Now publish live
        transport.publish("topic", b"live").await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), b"live");
    }

    #[tokio::test]
    async fn test_unsubscribe_then_resubscribe() {
        let transport = InMemoryTransport::new();
        transport.publish("topic", b"msg1").await.unwrap();

        let _rx = transport.subscribe("topic").await.unwrap();
        transport.unsubscribe("topic").await.unwrap();

        // Publish after unsubscribe — goes to history only
        transport.publish("topic", b"msg2").await.unwrap();

        // Re-subscribe should get full history
        let mut rx2 = transport.subscribe("topic").await.unwrap();
        assert_eq!(rx2.recv().await.unwrap(), b"msg1");
        assert_eq!(rx2.recv().await.unwrap(), b"msg2");
    }

    #[tokio::test]
    async fn test_unsubscribe_nonexistent_topic() {
        let transport = InMemoryTransport::new();
        // Should not panic
        transport.unsubscribe("never-subscribed").await.unwrap();
    }

    #[tokio::test]
    async fn test_default_trait() {
        let transport = InMemoryTransport::default();
        let mut rx = transport.subscribe("t").await.unwrap();
        transport.publish("t", b"default works").await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), b"default works");
    }

    #[tokio::test]
    async fn test_large_number_of_messages() {
        let transport = InMemoryTransport::new();
        let mut rx = transport.subscribe("bulk").await.unwrap();

        for i in 0u32..100 {
            transport.publish("bulk", &i.to_le_bytes()).await.unwrap();
        }

        for i in 0u32..100 {
            let msg = rx.recv().await.unwrap();
            assert_eq!(msg, i.to_le_bytes());
        }
    }

    #[tokio::test]
    async fn test_multiple_topics_independent_history() {
        let transport = InMemoryTransport::new();
        transport.publish("t1", b"a").await.unwrap();
        transport.publish("t2", b"b").await.unwrap();
        transport.publish("t1", b"c").await.unwrap();

        let mut rx1 = transport.subscribe("t1").await.unwrap();
        let mut rx2 = transport.subscribe("t2").await.unwrap();

        assert_eq!(rx1.recv().await.unwrap(), b"a");
        assert_eq!(rx1.recv().await.unwrap(), b"c");
        assert_eq!(rx2.recv().await.unwrap(), b"b");
        assert!(rx2.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_unsubscribe_one_topic_preserves_others() {
        let transport = InMemoryTransport::new();
        let _rx_a = transport.subscribe("a").await.unwrap();
        let mut rx_b = transport.subscribe("b").await.unwrap();

        transport.unsubscribe("a").await.unwrap();
        transport.publish("b", b"still alive").await.unwrap();

        assert_eq!(rx_b.recv().await.unwrap(), b"still alive");
    }

    #[tokio::test]
    async fn test_concurrent_publish_from_cloned_transports() {
        let transport = InMemoryTransport::new();
        let mut rx = transport.subscribe("topic").await.unwrap();

        let mut handles = Vec::new();
        for i in 0u32..10 {
            let t = transport.clone();
            handles.push(tokio::spawn(async move {
                t.publish("topic", &i.to_le_bytes()).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let mut received = Vec::new();
        for _ in 0..10 {
            received.push(rx.recv().await.unwrap());
        }
        // All 10 messages arrived (order may vary due to concurrency)
        assert_eq!(received.len(), 10);
    }

    #[tokio::test]
    async fn test_concurrent_subscribe_and_publish() {
        let transport = InMemoryTransport::new();

        // Spawn concurrent subscribers
        let mut sub_handles = Vec::new();
        for _ in 0..5 {
            let t = transport.clone();
            sub_handles.push(tokio::spawn(
                async move { t.subscribe("race").await.unwrap() },
            ));
        }
        let mut receivers: Vec<_> = Vec::new();
        for h in sub_handles {
            receivers.push(h.await.unwrap());
        }

        transport.publish("race", b"go").await.unwrap();

        // All concurrent subscribers should receive the message
        for rx in &mut receivers {
            assert_eq!(rx.recv().await.unwrap(), b"go");
        }
    }

    #[tokio::test]
    async fn test_large_payload() {
        let transport = InMemoryTransport::new();
        let mut rx = transport.subscribe("big").await.unwrap();

        let payload = vec![0xABu8; 1024 * 1024]; // 1 MiB
        transport.publish("big", &payload).await.unwrap();

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.len(), 1024 * 1024);
        assert!(msg.iter().all(|&b| b == 0xAB));
    }

    #[tokio::test]
    async fn test_binary_payload_roundtrip() {
        let transport = InMemoryTransport::new();
        let mut rx = transport.subscribe("bin").await.unwrap();

        // All possible byte values
        let payload: Vec<u8> = (0..=255).collect();
        transport.publish("bin", &payload).await.unwrap();

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg, payload);
    }

    #[tokio::test]
    async fn test_special_topic_names() {
        let transport = InMemoryTransport::new();

        for topic in &[
            "",
            "/waku/2/default/proto",
            "topic with spaces",
            "日本語トピック",
            "a".repeat(1000).as_str(),
        ] {
            let mut rx = transport.subscribe(topic).await.unwrap();
            transport.publish(topic, b"ok").await.unwrap();
            assert_eq!(rx.recv().await.unwrap(), b"ok");
        }
    }

    #[tokio::test]
    async fn test_subscribe_multiple_times_same_topic() {
        let transport = InMemoryTransport::new();
        let mut rx1 = transport.subscribe("dup").await.unwrap();
        let mut rx2 = transport.subscribe("dup").await.unwrap();
        let mut rx3 = transport.subscribe("dup").await.unwrap();

        transport.publish("dup", b"fanout").await.unwrap();

        assert_eq!(rx1.recv().await.unwrap(), b"fanout");
        assert_eq!(rx2.recv().await.unwrap(), b"fanout");
        assert_eq!(rx3.recv().await.unwrap(), b"fanout");
    }

    #[tokio::test]
    async fn test_history_order_preserved() {
        let transport = InMemoryTransport::new();
        for i in 0u32..50 {
            transport
                .publish("ordered", &i.to_le_bytes())
                .await
                .unwrap();
        }

        let mut rx = transport.subscribe("ordered").await.unwrap();
        for i in 0u32..50 {
            let msg = rx.recv().await.unwrap();
            assert_eq!(
                msg,
                i.to_le_bytes(),
                "history replay order violated at index {}",
                i
            );
        }
    }

    #[tokio::test]
    async fn test_unsubscribe_does_not_clear_history() {
        let transport = InMemoryTransport::new();
        transport.publish("persist", b"before").await.unwrap();
        let _rx = transport.subscribe("persist").await.unwrap();
        transport.unsubscribe("persist").await.unwrap();

        // History should still be available for new subscribers
        let mut rx2 = transport.subscribe("persist").await.unwrap();
        assert_eq!(rx2.recv().await.unwrap(), b"before");
    }

    #[tokio::test]
    async fn test_dropped_receiver_does_not_affect_other_subscribers() {
        let transport = InMemoryTransport::new();
        let rx1 = transport.subscribe("drop-test").await.unwrap();
        let mut rx2 = transport.subscribe("drop-test").await.unwrap();

        // Drop first subscriber
        drop(rx1);

        // Second subscriber should still work
        transport.publish("drop-test", b"survivor").await.unwrap();
        assert_eq!(rx2.recv().await.unwrap(), b"survivor");
    }
}
