//! Integration tests for message retry with exponential backoff.

use anyhow::Result;
use async_trait::async_trait;
use logos_messaging_a2a_core::RetryConfig;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::Transport;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// FailNTransport: fails the first N publishes, then succeeds
// ---------------------------------------------------------------------------

struct FailNState {
    fail_count: usize,
    subscribers: HashMap<String, Vec<mpsc::Sender<Vec<u8>>>>,
    history: HashMap<String, Vec<Vec<u8>>>,
}

#[derive(Clone)]
struct FailNTransport {
    state: Arc<Mutex<FailNState>>,
    attempts: Arc<AtomicUsize>,
}

impl FailNTransport {
    fn new(fail_count: usize) -> Self {
        let attempts = Arc::new(AtomicUsize::new(0));
        Self {
            state: Arc::new(Mutex::new(FailNState {
                fail_count,
                subscribers: HashMap::new(),
                history: HashMap::new(),
            })),
            attempts,
        }
    }

    fn attempt_count(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Transport for FailNTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        let fail_count = self.state.lock().unwrap().fail_count;

        if attempt < fail_count {
            return Err(anyhow::anyhow!(
                "simulated transport failure (attempt {})",
                attempt + 1
            ));
        }

        let data = payload.to_vec();
        let mut state = self.state.lock().unwrap();
        state
            .history
            .entry(topic.to_string())
            .or_default()
            .push(data.clone());
        if let Some(subs) = state.subscribers.get_mut(topic) {
            subs.retain(|tx| tx.try_send(data.clone()).is_ok());
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let mut state = self.state.lock().unwrap();
        let (tx, rx) = mpsc::channel(1024);
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
        let mut state = self.state.lock().unwrap();
        state.subscribers.remove(topic);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_retry_succeeds_after_failures() {
    let transport = FailNTransport::new(3); // fail 3 times, then succeed
    let attempts = transport.attempts.clone();

    let retry_cfg = RetryConfig {
        max_attempts: 5,
        base_delay_ms: 10, // fast for testing
        max_delay_ms: 100,
        jitter: false,
    };

    let node = WakuA2ANode::with_config(
        "retry-test",
        "retry test agent",
        vec![],
        transport,
        logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        },
    )
    .with_retry(retry_cfg);

    let task = logos_messaging_a2a_core::Task::new(node.pubkey(), "02deadbeef", "hello retry");
    let result = node.send_task(&task).await;
    assert!(result.is_ok(), "send should succeed after retries");

    // 3 failures + 1 success = 4 transport publish calls (at least)
    assert!(
        attempts.load(Ordering::SeqCst) >= 4,
        "expected at least 4 attempts, got {}",
        attempts.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn test_retry_exhausted() {
    let transport = FailNTransport::new(100); // always fail

    let retry_cfg = RetryConfig {
        max_attempts: 3,
        base_delay_ms: 10,
        max_delay_ms: 50,
        jitter: false,
    };

    let node = WakuA2ANode::with_config(
        "retry-exhaust",
        "retry exhaust agent",
        vec![],
        transport.clone(),
        logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        },
    )
    .with_retry(retry_cfg);

    let task = logos_messaging_a2a_core::Task::new(node.pubkey(), "02deadbeef", "doomed");
    let result = node.send_task(&task).await;
    assert!(result.is_err(), "send should fail when retries exhausted");

    // Should have attempted exactly 3 times
    assert_eq!(transport.attempt_count(), 3);
}

#[tokio::test]
async fn test_no_retry_without_config() {
    let transport = FailNTransport::new(1); // fail once

    // No retry config — should fail immediately
    let node = WakuA2ANode::with_config(
        "no-retry",
        "no retry agent",
        vec![],
        transport.clone(),
        logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        },
    );

    let task = logos_messaging_a2a_core::Task::new(node.pubkey(), "02deadbeef", "no retry");
    let result = node.send_task(&task).await;
    assert!(result.is_err(), "should fail without retry config");
    assert_eq!(transport.attempt_count(), 1, "should only attempt once");
}

#[tokio::test]
async fn test_retry_first_attempt_succeeds() {
    let transport = FailNTransport::new(0); // never fail

    let retry_cfg = RetryConfig {
        max_attempts: 5,
        base_delay_ms: 10,
        max_delay_ms: 100,
        jitter: false,
    };

    let node = WakuA2ANode::with_config(
        "instant-ok",
        "instant success agent",
        vec![],
        transport.clone(),
        logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        },
    )
    .with_retry(retry_cfg);

    let task = logos_messaging_a2a_core::Task::new(node.pubkey(), "02deadbeef", "easy");
    let result = node.send_task(&task).await;
    assert!(result.is_ok(), "should succeed on first attempt");
    // Only 1 publish attempt needed (the SDS layer may add ACK publishes)
    assert!(
        transport.attempt_count() >= 1,
        "should have at least 1 attempt"
    );
}

#[tokio::test]
async fn test_retry_with_jitter() {
    let transport = FailNTransport::new(2);
    let attempts = transport.attempts.clone();

    let retry_cfg = RetryConfig {
        max_attempts: 5,
        base_delay_ms: 10,
        max_delay_ms: 100,
        jitter: true, // enable jitter
    };

    let node = WakuA2ANode::with_config(
        "jitter-test",
        "jitter test agent",
        vec![],
        transport,
        logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        },
    )
    .with_retry(retry_cfg);

    let task = logos_messaging_a2a_core::Task::new(node.pubkey(), "02deadbeef", "jitter");
    let result = node.send_task(&task).await;
    assert!(result.is_ok(), "should succeed with jitter enabled");
    assert!(
        attempts.load(Ordering::SeqCst) >= 3,
        "expected at least 3 attempts"
    );
}

#[tokio::test]
async fn test_with_retry_builder() {
    let transport = FailNTransport::new(0);

    let node = WakuA2ANode::new("builder-test", "test", vec![], transport);
    assert!(node.retry_config().is_none());

    let transport2 = FailNTransport::new(0);
    let node2 = WakuA2ANode::new("builder-test2", "test", vec![], transport2)
        .with_retry(RetryConfig::default());
    assert!(node2.retry_config().is_some());
    assert_eq!(node2.retry_config().unwrap().max_attempts, 5);
}
