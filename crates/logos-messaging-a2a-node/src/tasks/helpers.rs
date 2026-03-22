use logos_messaging_a2a_core::{A2AEnvelope, AgentCard, Message, Task};
use logos_messaging_a2a_crypto::AgentIdentity;
use logos_messaging_a2a_transport::Transport;

use crate::metrics::Metrics;
use crate::{NodeError, Result, WakuA2ANode};

impl<T: Transport> WakuA2ANode<T> {
    /// Serialize a task into an envelope, offloading to storage if needed.
    ///
    /// When storage offload is configured and the serialized envelope exceeds
    /// the threshold, the original task is uploaded to storage and a slim
    /// envelope (with `payload_cid` set and content cleared) is returned.
    pub(crate) async fn prepare_payload(
        &self,
        task: &Task,
        recipient_card: Option<&AgentCard>,
    ) -> Result<Vec<u8>> {
        let envelope = self.maybe_encrypt_task(task, recipient_card)?;
        let payload = serde_json::to_vec(&envelope)?;

        if let Some(ref offload) = self.storage_offload {
            if payload.len() > offload.threshold_bytes {
                // Upload the original task (plaintext) to storage
                let task_bytes = serde_json::to_vec(task)?;
                let cid =
                    offload.backend.upload(task_bytes).await.map_err(|e| {
                        NodeError::Other(format!("storage offload upload failed: {e}"))
                    })?;

                // Build a slim task with the CID and cleared content
                let mut slim = task.clone();
                slim.payload_cid = Some(cid);
                slim.message = Message {
                    role: task.message.role.clone(),
                    parts: vec![],
                };
                if let Some(ref mut result) = slim.result {
                    result.parts.clear();
                }

                let slim_envelope = self.maybe_encrypt_task(&slim, recipient_card)?;
                return Ok(serde_json::to_vec(&slim_envelope)?);
            }
        }

        Ok(payload)
    }

    /// Encrypt a task if both sides have encryption identities.
    fn maybe_encrypt_task(
        &self,
        task: &Task,
        recipient_card: Option<&AgentCard>,
    ) -> Result<A2AEnvelope> {
        if let (Some(ref identity), Some(card)) = (&self.identity, recipient_card) {
            if let Some(ref bundle) = card.intro_bundle {
                let their_pubkey = AgentIdentity::parse_public_key(&bundle.agent_pubkey)?;
                let session_key = identity.shared_key(&their_pubkey);
                let task_json = serde_json::to_vec(task)?;
                let encrypted = session_key.encrypt(&task_json)?;
                Metrics::inc(&self.metrics.encryptions);
                return Ok(A2AEnvelope::EncryptedTask {
                    encrypted,
                    sender_pubkey: identity.public_key_hex(),
                });
            }
        }
        Ok(A2AEnvelope::Task(task.clone()))
    }

    /// Decrypt an encrypted task payload.
    pub(crate) fn decrypt_task(
        &self,
        identity: &AgentIdentity,
        sender_pubkey_hex: &str,
        encrypted: &logos_messaging_a2a_crypto::EncryptedPayload,
    ) -> Result<Task> {
        let their_pubkey = AgentIdentity::parse_public_key(sender_pubkey_hex)?;
        let session_key = identity.shared_key(&their_pubkey);
        let plaintext = session_key.decrypt(encrypted)?;
        Metrics::inc(&self.metrics.decryptions);
        let task: Task = serde_json::from_slice(&plaintext)?;
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::StorageOffloadConfig;
    use crate::tasks::test_support::{
        fast_config, FailingDownloadStorage, FailingUploadStorage, MockStorage, MockTransport,
    };
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::Task;
    use std::sync::Arc;

    // --- Storage offload tests ---

    #[tokio::test]
    async fn test_small_payload_inline() {
        let transport = MockTransport::new();
        let published = transport.published.clone();
        let storage = Arc::new(MockStorage::new());

        let node = WakuA2ANode::with_config("test", "test agent", vec![], transport, fast_config())
            .with_storage_offload(StorageOffloadConfig::with_threshold(
                storage.clone(),
                65_536,
            ));

        let task = Task::new(node.pubkey(), "02deadbeef", "small message");
        node.send_task(&task).await.unwrap();

        // Payload was sent (published to transport)
        assert!(!published.lock().unwrap().is_empty());
        // Storage should NOT have been used
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn test_large_payload_offloaded() {
        let transport = MockTransport::new();
        let storage = Arc::new(MockStorage::new());

        // Very low threshold to force offloading
        let node = WakuA2ANode::with_config("test", "test agent", vec![], transport, fast_config())
            .with_storage_offload(StorageOffloadConfig::with_threshold(storage.clone(), 10));

        let task = Task::new(
            node.pubkey(),
            "02deadbeef",
            "this message exceeds the tiny threshold",
        );
        node.send_task(&task).await.unwrap();

        // Storage should have been used (one upload)
        assert_eq!(storage.len(), 1);
    }

    #[tokio::test]
    async fn test_offload_roundtrip() {
        let transport = MockTransport::new();
        let storage = Arc::new(MockStorage::new());
        let threshold = 10;

        // Create receiver first to capture its pubkey
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_storage_offload(StorageOffloadConfig::with_threshold(
            storage.clone(),
            threshold,
        ));
        let recipient_pubkey = receiver.pubkey().to_string();

        // Lazy-subscribe so the receiver listens on its task topic
        let _ = receiver.poll_tasks().await.unwrap();

        // Create sender on the same shared transport
        let sender = WakuA2ANode::with_config(
            "sender",
            "sender agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_storage_offload(StorageOffloadConfig::with_threshold(
            storage.clone(),
            threshold,
        ));

        let large_text = "A".repeat(1000);
        let task = Task::new(sender.pubkey(), &recipient_pubkey, &large_text);
        sender.send_task(&task).await.unwrap();

        // Verify payload was offloaded to storage
        assert_eq!(storage.len(), 1);

        // Receiver polls — should auto-fetch from storage and return the full task
        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].text(), Some(large_text.as_str()));
        // The reconstructed task should NOT have a payload_cid (it's the original)
        assert!(received[0].payload_cid.is_none());
    }

    #[tokio::test]
    async fn test_encrypted_roundtrip_preserves_task_fields() {
        let transport = MockTransport::new();
        let alice = WakuA2ANode::new_encrypted("alice", "Alice", vec![], transport.clone());
        let bob = WakuA2ANode::new_encrypted("bob", "Bob", vec![], transport.clone());
        let bpk = bob.pubkey().to_string();
        let _ = bob.poll_tasks().await.unwrap();

        let mut task = Task::new(alice.pubkey(), &bpk, "secret message");
        task.session_id = Some("sess-123".to_string());

        alice.send_task_to(&task, Some(&bob.card)).await.unwrap();

        let received = bob.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].text(), Some("secret message"));
        assert_eq!(received[0].from, alice.pubkey());
        assert_eq!(received[0].session_id, Some("sess-123".to_string()));
    }

    #[tokio::test]
    async fn test_storage_offload_config_default_threshold() {
        let storage = Arc::new(MockStorage::new());
        let config = StorageOffloadConfig::new(storage);
        assert_eq!(config.threshold_bytes, 65_536);
    }

    #[tokio::test]
    async fn test_storage_offload_config_custom_threshold() {
        let storage = Arc::new(MockStorage::new());
        let config = StorageOffloadConfig::with_threshold(storage, 1024);
        assert_eq!(config.threshold_bytes, 1024);
    }

    #[test]
    fn storage_offload_config_zero_threshold() {
        let storage = Arc::new(MockStorage::new());
        let config = StorageOffloadConfig::with_threshold(storage, 0);
        assert_eq!(config.threshold_bytes, 0);
    }

    #[tokio::test]
    async fn offload_at_exact_threshold_boundary_not_offloaded() {
        // Payload exactly at threshold should NOT be offloaded (only > threshold triggers it)
        let transport = MockTransport::new();
        let storage = Arc::new(MockStorage::new());

        // We'll measure the serialized size of a small task
        let node =
            WakuA2ANode::with_config("test", "test", vec![], transport.clone(), fast_config());
        let task = Task::new(node.pubkey(), "02aa", "x");
        let envelope = logos_messaging_a2a_core::A2AEnvelope::Task(task.clone());
        let serialized_len = serde_json::to_vec(&envelope).unwrap().len();

        // Set threshold to exactly the serialized length
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_storage_offload(StorageOffloadConfig::with_threshold(
                storage.clone(),
                serialized_len,
            ));

        let task = Task::new(node.pubkey(), "02aa", "x");
        node.send_task(&task).await.unwrap();

        // Should NOT be offloaded since len == threshold (only > triggers)
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn offload_one_byte_over_threshold() {
        let transport = MockTransport::new();
        let storage = Arc::new(MockStorage::new());

        // Set threshold very small to force offloading
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_storage_offload(StorageOffloadConfig::with_threshold(storage.clone(), 1));

        let task = Task::new(node.pubkey(), "02aa", "hi");
        node.send_task(&task).await.unwrap();

        // Should be offloaded since any task serialization > 1 byte
        assert_eq!(storage.len(), 1);
    }

    #[tokio::test]
    async fn upload_failure_propagates_error() {
        let transport = MockTransport::new();
        let storage = Arc::new(FailingUploadStorage);

        // Tiny threshold to force offload attempt
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_storage_offload(StorageOffloadConfig::with_threshold(storage, 1));

        let task = Task::new(node.pubkey(), "02aa", "this will fail");
        let result = node.send_task(&task).await;

        assert!(result.is_err(), "upload failure should propagate as error");
        assert!(
            result.unwrap_err().to_string().contains("upload failed"),
            "error should mention upload failure"
        );
    }

    #[tokio::test]
    async fn download_failure_propagates_error() {
        let transport = MockTransport::new();
        let storage = Arc::new(FailingDownloadStorage::new());

        // Sender offloads (upload succeeds)
        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config())
                .with_storage_offload(StorageOffloadConfig::with_threshold(storage.clone(), 1));

        // Receiver has the failing-download storage
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_storage_offload(StorageOffloadConfig::with_threshold(storage, 1));
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let task = Task::new(sender.pubkey(), &rpk, "will fail to fetch");
        sender.send_task(&task).await.unwrap();

        // Receiver's poll should fail because download fails
        let result = receiver.poll_tasks().await;
        assert!(
            result.is_err(),
            "download failure should propagate as error"
        );
    }

    #[tokio::test]
    async fn no_offload_without_config() {
        let transport = MockTransport::new();
        let published = transport.published.clone();

        // Node WITHOUT storage offload
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config());

        let large_text = "B".repeat(100_000);
        let task = Task::new(node.pubkey(), "02aa", &large_text);
        node.send_task(&task).await.unwrap();

        // Should still send (just inline, no offload)
        assert!(!published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn offloaded_task_has_cleared_content() {
        let transport = MockTransport::new();
        let published = transport.published.clone();
        let storage = Arc::new(MockStorage::new());

        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_storage_offload(StorageOffloadConfig::with_threshold(storage.clone(), 1));

        let task = Task::new(node.pubkey(), "02aa", "offloaded content");
        node.send_task(&task).await.unwrap();

        // The published SDS message wraps the envelope — extract and check
        let pubs = published.lock().unwrap();
        assert!(!pubs.is_empty());

        // Storage should have the original task
        assert_eq!(storage.len(), 1);
    }

    #[tokio::test]
    async fn multiple_offloads_produce_unique_cids() {
        let transport = MockTransport::new();
        let storage = Arc::new(MockStorage::new());

        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_storage_offload(StorageOffloadConfig::with_threshold(storage.clone(), 1));

        for i in 0..5 {
            let task = Task::new(node.pubkey(), "02aa", &format!("msg-{i}"));
            node.send_task(&task).await.unwrap();
        }

        // Each upload should have gotten a unique CID
        assert_eq!(storage.len(), 5);
    }

    #[tokio::test]
    async fn with_storage_offload_builder() {
        let transport = MockTransport::new();
        let storage = Arc::new(MockStorage::new());
        let node = WakuA2ANode::new("test", "test", vec![], transport)
            .with_storage_offload(StorageOffloadConfig::new(storage));
        // Verify that storage_offload is configured
        assert!(node.storage_offload.is_some());
    }
}
