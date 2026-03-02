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
}
