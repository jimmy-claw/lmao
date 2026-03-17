use logos_messaging_a2a_core::{A2AEnvelope, Task};
use logos_messaging_a2a_transport::Transport;

use crate::session::Session;
use crate::{NodeError, Result, WakuA2ANode};

impl<T: Transport> WakuA2ANode<T> {
    /// Poll for incoming tasks addressed to this agent.
    ///
    /// Lazily subscribes to the task topic on first call. Processes incoming
    /// messages through the SDS MessageChannel for:
    /// - Bloom filter deduplication
    /// - Causal ordering (buffering out-of-order messages)
    /// - Lamport timestamp synchronization
    /// - Implicit ACK via bloom filter exchange
    ///
    /// Also sends explicit ACKs using the SDS message_id so that the sender's
    /// `send_reliable` retransmission loop can terminate.
    ///
    /// Automatically decrypts encrypted tasks if this node has an identity.
    pub async fn poll_tasks(&self) -> Result<Vec<Task>> {
        let raw_messages = {
            let mut task_rx = self.task_rx.lock().await;
            if task_rx.is_none() {
                let topic = logos_messaging_a2a_core::topics::task_topic(&self.card.public_key);
                *task_rx = Some(self.channel.transport().subscribe(&topic).await?);
            }
            let rx = task_rx.as_mut().unwrap();

            let mut msgs = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                msgs.push(msg);
            }
            msgs
        };

        let mut tasks = Vec::new();

        for msg in raw_messages {
            // Try to process as SDS message (causal ordering + dedup + bloom)
            let delivered_content = self.channel.receive(&msg);

            if !delivered_content.is_empty() {
                // SDS-wrapped messages: extract tasks from delivered content
                for content in delivered_content {
                    // Send explicit ACK using the SDS message_id so the sender's
                    // send_reliable() retransmission terminates promptly.
                    let _ = self.channel.send_ack("", &content.message_id).await;

                    if let Some(task) = self.extract_task(&content.content).await? {
                        tasks.push(task);
                    }
                }
            } else {
                // Backward compat: try raw A2AEnvelope (non-SDS peers)
                if let Ok(envelope) = serde_json::from_slice::<A2AEnvelope>(&msg) {
                    // Dedup via bloom filter using a hash of the raw bytes
                    let dedup_id = logos_messaging_a2a_transport::sds::compute_message_id(&msg);
                    if self.channel.is_duplicate(&dedup_id) {
                        continue;
                    }
                    self.channel.bloom.set(&dedup_id);

                    if let Some(task) = self.extract_task_from_envelope(envelope).await? {
                        tasks.push(task);
                    }
                }
            }
        }
        // Reject tasks that don't meet the payment requirement
        // Verify payments (async — can't use retain)
        let mut verified_tasks = Vec::new();
        for task in tasks {
            if self.verify_payment(&task).await {
                verified_tasks.push(task);
            }
        }
        let tasks = verified_tasks;

        self.metrics.inc_tasks_received(tasks.len() as u64);

        // Track incoming tasks in their sessions
        for task in &tasks {
            if let Some(ref sid) = task.session_id {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .entry(sid.clone())
                    .or_insert_with(|| Session::new(&task.from));
                if !session.task_ids.contains(&task.id) {
                    session.task_ids.push(task.id.clone());
                }
            }
        }
        Ok(tasks)
    }

    /// Extract a task from raw payload bytes (inner content of an SDS message).
    async fn extract_task(&self, payload: &[u8]) -> Result<Option<Task>> {
        if let Ok(envelope) = serde_json::from_slice::<A2AEnvelope>(payload) {
            self.extract_task_from_envelope(envelope).await
        } else {
            Ok(None)
        }
    }

    /// Extract a task from an A2AEnvelope, handling decryption and CID fetching.
    async fn extract_task_from_envelope(&self, envelope: A2AEnvelope) -> Result<Option<Task>> {
        match envelope {
            A2AEnvelope::Task(task) => self.maybe_fetch_offloaded(task).await,
            A2AEnvelope::EncryptedTask {
                encrypted,
                sender_pubkey,
            } => {
                if let Some(ref identity) = self.identity {
                    match self.decrypt_task(identity, &sender_pubkey, &encrypted) {
                        Ok(task) => self.maybe_fetch_offloaded(task).await,
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to decrypt task");
                            Ok(None)
                        }
                    }
                } else {
                    tracing::warn!("Received encrypted task but no identity configured");
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// If the task has a `payload_cid`, fetch the full payload from storage.
    async fn maybe_fetch_offloaded(&self, task: Task) -> Result<Option<Task>> {
        if let Some(ref cid) = task.payload_cid {
            if let Some(ref offload) = self.storage_offload {
                let data =
                    offload.backend.download(cid).await.map_err(|e| {
                        NodeError::Other(format!("storage fetch by CID failed: {e}"))
                    })?;
                let original: Task = serde_json::from_slice(&data)?;
                return Ok(Some(original));
            }
            tracing::warn!(payload_cid = %cid, "Task has payload_cid but no storage backend configured");
        }
        Ok(Some(task))
    }
}

#[cfg(test)]
mod tests {
    use crate::tasks::test_support::{fast_config, MockTransport};
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard, Task};
    use logos_messaging_a2a_transport::Transport;

    #[tokio::test]
    async fn test_poll_tasks() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("echo", "echo agent", vec!["text".into()], transport);

        let tasks = node.poll_tasks().await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn test_malformed_json_ignored() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config(
            "test",
            "test agent",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let topic = topics::task_topic(node.pubkey());

        // Subscribe first
        let _ = node.poll_tasks().await.unwrap();

        // Inject garbage bytes
        transport.publish(&topic, b"not json at all").await.unwrap();
        transport.publish(&topic, b"{malformed}").await.unwrap();
        transport.publish(&topic, b"").await.unwrap();

        // Should not crash, just return empty
        let tasks = node.poll_tasks().await.unwrap();
        assert!(
            tasks.is_empty(),
            "malformed messages should be silently ignored"
        );
    }

    #[tokio::test]
    async fn test_wrong_envelope_type_ignored() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config(
            "test",
            "test agent",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let topic = topics::task_topic(node.pubkey());
        let _ = node.poll_tasks().await.unwrap();

        // Send an AgentCard envelope on the task topic — should be ignored
        let card_envelope = A2AEnvelope::AgentCard(AgentCard {
            name: "impostor".to_string(),
            description: "not a task".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![],
            public_key: "02dead".to_string(),
            intro_bundle: None,
        });
        let payload = serde_json::to_vec(&card_envelope).unwrap();
        transport.publish(&topic, &payload).await.unwrap();

        let tasks = node.poll_tasks().await.unwrap();
        assert!(
            tasks.is_empty(),
            "non-Task envelopes should not produce tasks"
        );
    }

    #[tokio::test]
    async fn test_ack_envelope_on_task_topic_ignored() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config(
            "test",
            "test agent",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let topic = topics::task_topic(node.pubkey());
        let _ = node.poll_tasks().await.unwrap();

        let ack_envelope = A2AEnvelope::Ack {
            message_id: "fake-msg-id".to_string(),
        };
        let payload = serde_json::to_vec(&ack_envelope).unwrap();
        transport.publish(&topic, &payload).await.unwrap();

        let tasks = node.poll_tasks().await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn test_encrypted_task_without_identity_ignored() {
        let transport = MockTransport::new();
        // Node WITHOUT encryption
        let node = WakuA2ANode::with_config(
            "test",
            "test agent",
            vec![],
            transport.clone(),
            fast_config(),
        );
        assert!(node.identity().is_none());
        let topic = topics::task_topic(node.pubkey());
        let _ = node.poll_tasks().await.unwrap();

        // Send an encrypted task envelope
        let enc_envelope = A2AEnvelope::EncryptedTask {
            encrypted: logos_messaging_a2a_crypto::EncryptedPayload {
                nonce: "AAAAAAAAAAAAAAAA".to_string(),
                ciphertext: "Y2lwaGVydGV4dA==".to_string(),
            },
            sender_pubkey: "02aabbccdd".to_string(),
        };
        let payload = serde_json::to_vec(&enc_envelope).unwrap();
        transport.publish(&topic, &payload).await.unwrap();

        // Should silently skip — no identity to decrypt
        let tasks = node.poll_tasks().await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn test_poll_tasks_returns_empty_on_repeated_calls() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test agent", vec![], transport, fast_config());

        // Multiple polls on empty topic
        for _ in 0..5 {
            let tasks = node.poll_tasks().await.unwrap();
            assert!(tasks.is_empty());
        }
    }

    #[tokio::test]
    async fn test_task_with_payload_cid_but_no_storage_backend() {
        let transport = MockTransport::new();
        // Node WITHOUT storage offload configured
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        // Manually inject a task envelope with payload_cid set
        let mut task = Task::new("02sender", &rpk, "placeholder");
        task.payload_cid = Some("zQmNoBackend".to_string());
        let envelope = A2AEnvelope::Task(task);
        let payload = serde_json::to_vec(&envelope).unwrap();
        let topic = topics::task_topic(&rpk);
        transport.publish(&topic, &payload).await.unwrap();

        // Should still return the task (with the placeholder text), just can't fetch CID
        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].payload_cid, Some("zQmNoBackend".to_string()));
    }

    #[tokio::test]
    async fn test_encrypted_node_decrypt_with_wrong_sender_pubkey_ignored() {
        let transport = MockTransport::new();
        let receiver =
            WakuA2ANode::new_encrypted("receiver", "receiver", vec![], transport.clone());
        let topic = topics::task_topic(receiver.pubkey());
        let _ = receiver.poll_tasks().await.unwrap();

        // Create an encrypted task envelope with a bogus sender_pubkey
        // that doesn't match any actual identity — decryption should fail
        let alice = logos_messaging_a2a_crypto::AgentIdentity::generate();
        let bogus_key = logos_messaging_a2a_crypto::AgentIdentity::generate();

        // Encrypt with alice's key agreement to receiver, but claim sender is bogus
        let receiver_identity = receiver.identity().unwrap();
        let shared = alice.shared_key(&receiver_identity.public);
        let task = Task::new("02fake", receiver.pubkey(), "sneaky");
        let task_json = serde_json::to_vec(&task).unwrap();
        let encrypted = shared.encrypt(&task_json).unwrap();

        let envelope = A2AEnvelope::EncryptedTask {
            encrypted,
            sender_pubkey: bogus_key.public_key_hex(), // wrong sender
        };
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(&topic, &payload).await.unwrap();

        // Receiver tries to decrypt using bogus_key as sender — ECDH will produce wrong shared secret
        let tasks = receiver.poll_tasks().await.unwrap();
        assert!(
            tasks.is_empty(),
            "wrong sender pubkey should cause decryption failure"
        );
    }

    #[tokio::test]
    async fn test_session_auto_created_on_incoming_task_with_session_id() {
        let transport = MockTransport::new();
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let spk = sender.pubkey().to_string();

        // Create a session on sender side and send within it
        let session = sender.create_session(&rpk);
        sender
            .send_in_session(&session.id, "hi from session")
            .await
            .unwrap();

        // Receiver polls — session should be auto-created
        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].session_id, Some(session.id.clone()));

        // Receiver should now have a session for this
        let recv_session = receiver.get_session(&session.id);
        assert!(
            recv_session.is_some(),
            "session should be auto-created on receive"
        );
        assert_eq!(recv_session.unwrap().peer, spk);
    }

    #[tokio::test]
    async fn incoming_session_task_tracked_in_existing_session() {
        let transport = MockTransport::new();
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // Receiver pre-creates a session
        let session = receiver.create_session(sender.pubkey());

        // Sender sends a task with that session ID
        let task = Task::new_in_session(sender.pubkey(), &rpk, "within session", &session.id);
        sender.send_task(&task).await.unwrap();

        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        // The task should be tracked in the existing session
        let updated = receiver.get_session(&session.id).unwrap();
        assert!(updated.task_ids.contains(&task.id));
    }

    #[tokio::test]
    async fn incoming_task_without_session_id_not_tracked() {
        let transport = MockTransport::new();
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // Send a task without session_id
        let task = Task::new(sender.pubkey(), &rpk, "no session");
        sender.send_task(&task).await.unwrap();

        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].session_id.is_none());

        // No sessions should be created
        assert!(receiver.list_sessions().is_empty());
    }
}
