use anyhow::{Context, Result};
use logos_messaging_a2a_core::{topics, AgentCard, Task};
use logos_messaging_a2a_transport::Transport;

use crate::retry;
use crate::WakuA2ANode;

impl<T: Transport> WakuA2ANode<T> {
    /// Send a task to another agent. Uses SDS reliable delivery with
    /// causal ordering, bloom filter, and retransmission.
    pub async fn send_task(&self, task: &Task) -> Result<bool> {
        self.send_task_to(task, None).await
    }

    /// Send a task, optionally encrypting if recipient has an intro bundle.
    ///
    /// When a [`PaymentConfig`](crate::PaymentConfig) with `auto_pay = true` is set, the node
    /// calls `backend.pay()` before sending and attaches the TX hash to
    /// the task envelope.
    ///
    /// When a [`RetryConfig`](logos_messaging_a2a_core::RetryConfig) is set (via [`with_retry`](Self::with_retry)),
    /// transport-level failures are retried with exponential backoff.
    pub async fn send_task_to(
        &self,
        task: &Task,
        recipient_card: Option<&AgentCard>,
    ) -> Result<bool> {
        let task = self.maybe_auto_pay(task).await?;
        let topic = topics::task_topic(&task.to);
        let payload = self.prepare_payload(&task, recipient_card).await?;

        // Use SDS reliable delivery — the SDS message_id (SHA256 of payload)
        // is used for ACK routing, not the task UUID.
        let (_msg, acked) = if let Some(ref retry_cfg) = self.retry_config {
            retry::RetryLayer::new(&self.channel, retry_cfg)
                .send_reliable(&topic, &payload)
                .await
                .context("SDS publish failed (after retries)")?
        } else {
            self.channel
                .send_reliable(&topic, &payload)
                .await
                .context("SDS publish failed")?
        };

        if acked {
            tracing::info!(task_id = %task.id, "Task sent and ACKed");
        } else {
            tracing::warn!(task_id = %task.id, "Task sent but no ACK received");
        }
        Ok(acked)
    }

    /// Send a text message within an existing session.
    pub async fn send_in_session(&self, session_id: &str, text: &str) -> Result<Task> {
        let peer = {
            let sessions = self.sessions.lock().unwrap();
            let session = sessions
                .get(session_id)
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?;
            session.peer.clone()
        };
        let task = Task::new_in_session(self.pubkey(), &peer, text, session_id);
        self.send_task(&task).await?;
        if let Some(s) = self.sessions.lock().unwrap().get_mut(session_id) {
            s.task_ids.push(task.id.clone());
        }
        Ok(task)
    }

    /// Create a task and send it.
    pub async fn send_text(&self, to: &str, text: &str) -> Result<Task> {
        let task = Task::new(self.pubkey(), to, text);
        self.send_task(&task).await?;
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use crate::tasks::test_support::{fast_config, MockTransport};
    use crate::WakuA2ANode;

    #[tokio::test]
    async fn test_send_text_creates_and_sends_task() {
        let transport = MockTransport::new();
        let published = transport.published.clone();
        let node =
            WakuA2ANode::with_config("sender", "sender node", vec![], transport, fast_config());

        let task = node.send_text("02deadbeef", "hello world").await.unwrap();
        assert_eq!(task.from, node.pubkey());
        assert_eq!(task.to, "02deadbeef");
        assert_eq!(task.text(), Some("hello world"));
        assert!(!published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_send_in_session() {
        let transport = MockTransport::new();
        let published = transport.published.clone();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        let session = node.create_session("02deadbeef");
        let task = node.send_in_session(&session.id, "hello").await.unwrap();
        assert_eq!(task.session_id, Some(session.id.clone()));
        assert_eq!(task.to, "02deadbeef");
        assert!(!published.lock().unwrap().is_empty());

        // Task should be tracked in session
        let updated = node.get_session(&session.id).unwrap();
        assert_eq!(updated.task_ids.len(), 1);
        assert_eq!(updated.task_ids[0], task.id);
    }

    #[tokio::test]
    async fn test_send_in_nonexistent_session() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        let result = node.send_in_session("nonexistent", "hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_in_nonexistent_session_error_message() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        let err = node
            .send_in_session("ghost-session", "hi")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("ghost-session"),
            "error should mention the session ID"
        );
    }

    #[tokio::test]
    async fn test_send_text_received_by_peer() {
        let transport = MockTransport::new();

        let alice = WakuA2ANode::with_config(
            "alice",
            "Alice",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let apk = alice.pubkey().to_string();

        let bob = WakuA2ANode::with_config(
            "bob",
            "Bob",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let bpk = bob.pubkey().to_string();
        let _ = bob.poll_tasks().await.unwrap(); // subscribe

        let task = alice.send_text(&bpk, "hey bob").await.unwrap();
        assert_eq!(task.from, apk);
        assert_eq!(task.to, bpk);

        let received = bob.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].text(), Some("hey bob"));
        assert_eq!(received[0].from, apk);
    }
}
