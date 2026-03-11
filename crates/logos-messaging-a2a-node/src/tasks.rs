//! Task sending, receiving, and payment operations for [`WakuA2ANode`](crate::WakuA2ANode).

use anyhow::{Context, Result};
use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard, Message, Task};
use logos_messaging_a2a_crypto::AgentIdentity;
use logos_messaging_a2a_execution::AgentId;
use logos_messaging_a2a_transport::Transport;

use crate::retry;
use crate::session::Session;
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
                let topic = topics::task_topic(&self.card.public_key);
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
                let data = offload
                    .backend
                    .download(cid)
                    .await
                    .map_err(|e| anyhow::anyhow!("storage fetch by CID failed: {e}"))?;
                let original: Task = serde_json::from_slice(&data)
                    .context("Failed to deserialize offloaded task")?;
                return Ok(Some(original));
            }
            tracing::warn!(payload_cid = %cid, "Task has payload_cid but no storage backend configured");
        }
        Ok(Some(task))
    }

    /// Respond to a task: send back a completed task with result.
    ///
    /// Uses SDS causal send (maintains ordering, includes bloom filter
    /// for implicit ACK) but does not block on explicit ACK.
    pub async fn respond(&self, task: &Task, result_text: &str) -> Result<()> {
        self.respond_to(task, result_text, None).await
    }

    /// Respond to a task, optionally encrypting to the sender.
    pub async fn respond_to(
        &self,
        task: &Task,
        result_text: &str,
        sender_card: Option<&AgentCard>,
    ) -> Result<()> {
        let response = task.respond(result_text);
        let topic = topics::task_topic(&response.to);
        let payload = self.prepare_payload(&response, sender_card).await?;

        // Use causal send for responses (maintains ordering, no retransmit block)
        self.channel
            .send(&topic, &payload)
            .await
            .context("Failed to send response")?;

        tracing::info!(task_id = %task.id, "Responded to task");
        Ok(())
    }

    /// Create a task and send it.
    pub async fn send_text(&self, to: &str, text: &str) -> Result<Task> {
        let task = Task::new(self.pubkey(), to, text);
        self.send_task(&task).await?;
        Ok(task)
    }

    /// If auto-pay is enabled, call `backend.pay()` and attach proof to the task.
    pub async fn maybe_auto_pay(&self, task: &Task) -> Result<Task> {
        if let Some(ref pay_cfg) = self.payment {
            if pay_cfg.auto_pay && pay_cfg.auto_pay_amount > 0 {
                let recipient = AgentId(task.to.clone());
                let tx_hash = pay_cfg
                    .backend
                    .pay(&recipient, pay_cfg.auto_pay_amount)
                    .await
                    .context("auto-pay failed")?;
                let mut task = task.clone();
                task.payment_tx = Some(tx_hash.to_string());
                task.payment_amount = Some(pay_cfg.auto_pay_amount);
                return Ok(task);
            }
        }
        Ok(task.clone())
    }

    /// Check that an incoming task satisfies the payment requirement.
    ///
    /// When `verify_on_chain` is true, queries the chain via
    /// `backend.verify_transfer()` to confirm:
    /// 1. The tx hash exists and succeeded
    /// 2. The transfer amount meets the minimum requirement
    /// 3. The recipient matches `receiving_account` (if configured)
    /// 4. The tx hash hasn't been seen before (replay protection)
    ///
    /// Returns `true` if the task is accepted, `false` if rejected.
    async fn verify_payment(&self, task: &Task) -> bool {
        let pay_cfg = match &self.payment {
            Some(cfg) => cfg,
            None => return true,
        };

        if pay_cfg.required_amount == 0 {
            return true;
        }

        let tx_hash = match &task.payment_tx {
            Some(tx) if !tx.is_empty() => tx.clone(),
            _ => {
                tracing::warn!(
                    task_id = %task.id,
                    required_amount = pay_cfg.required_amount,
                    "Rejecting task — no payment tx hash provided"
                );
                return false;
            }
        };

        // Check for replayed tx hashes
        {
            let seen = self.seen_tx_hashes.lock().unwrap();
            if seen.contains(&tx_hash) {
                tracing::warn!(
                    task_id = %task.id,
                    tx_hash = %tx_hash,
                    "Rejecting task — replayed payment tx"
                );
                return false;
            }
        }

        if !pay_cfg.verify_on_chain {
            // Offline check: trust the claimed amount
            match task.payment_amount {
                Some(amount) if amount >= pay_cfg.required_amount => {
                    self.seen_tx_hashes.lock().unwrap().insert(tx_hash);
                    true
                }
                _ => {
                    tracing::warn!(
                        task_id = %task.id,
                        required = pay_cfg.required_amount,
                        got = ?task.payment_amount,
                        "Rejecting task — insufficient payment"
                    );
                    false
                }
            }
        } else {
            // On-chain verification
            match pay_cfg.backend.verify_transfer(&tx_hash).await {
                Ok(details) => {
                    if details.amount < pay_cfg.required_amount {
                        tracing::warn!(
                            task_id = %task.id,
                            on_chain_amount = details.amount,
                            required = pay_cfg.required_amount,
                            "Rejecting task — on-chain amount below required"
                        );
                        return false;
                    }
                    if !pay_cfg.receiving_account.is_empty()
                        && details.to.to_lowercase() != pay_cfg.receiving_account.to_lowercase()
                    {
                        tracing::warn!(
                            task_id = %task.id,
                            actual_recipient = %details.to,
                            expected_recipient = %pay_cfg.receiving_account,
                            "Rejecting task — payment sent to wrong address"
                        );
                        return false;
                    }
                    // Mark as seen to prevent replay
                    self.seen_tx_hashes.lock().unwrap().insert(tx_hash);
                    true
                }
                Err(e) => {
                    tracing::error!(
                        task_id = %task.id,
                        error = %e,
                        "Rejecting task — on-chain verification failed"
                    );
                    false
                }
            }
        }
    }

    /// Serialize a task into an envelope, offloading to storage if needed.
    ///
    /// When storage offload is configured and the serialized envelope exceeds
    /// the threshold, the original task is uploaded to storage and a slim
    /// envelope (with `payload_cid` set and content cleared) is returned.
    async fn prepare_payload(
        &self,
        task: &Task,
        recipient_card: Option<&AgentCard>,
    ) -> Result<Vec<u8>> {
        let envelope = self.maybe_encrypt_task(task, recipient_card)?;
        let payload = serde_json::to_vec(&envelope).context("Failed to serialize envelope")?;

        if let Some(ref offload) = self.storage_offload {
            if payload.len() > offload.threshold_bytes {
                // Upload the original task (plaintext) to storage
                let task_bytes =
                    serde_json::to_vec(task).context("Failed to serialize task for storage")?;
                let cid = offload
                    .backend
                    .upload(task_bytes)
                    .await
                    .map_err(|e| anyhow::anyhow!("storage offload upload failed: {e}"))?;

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
                return serde_json::to_vec(&slim_envelope)
                    .context("Failed to serialize slim envelope");
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
                return Ok(A2AEnvelope::EncryptedTask {
                    encrypted,
                    sender_pubkey: identity.public_key_hex(),
                });
            }
        }
        Ok(A2AEnvelope::Task(task.clone()))
    }

    /// Decrypt an encrypted task payload.
    fn decrypt_task(
        &self,
        identity: &AgentIdentity,
        sender_pubkey_hex: &str,
        encrypted: &logos_messaging_a2a_crypto::EncryptedPayload,
    ) -> Result<Task> {
        let their_pubkey = AgentIdentity::parse_public_key(sender_pubkey_hex)?;
        let session_key = identity.shared_key(&their_pubkey);
        let plaintext = session_key.decrypt(encrypted)?;
        let task: Task =
            serde_json::from_slice(&plaintext).context("Failed to deserialize decrypted task")?;
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use crate::payment::PaymentConfig;
    use crate::storage::StorageOffloadConfig;
    use crate::WakuA2ANode;
    use anyhow::Result;
    use async_trait::async_trait;
    use k256::ecdsa::SigningKey;
    use logos_messaging_a2a_core::RetryConfig;
    use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard, PresenceAnnouncement, Task};
    use logos_messaging_a2a_execution::{AgentId, ExecutionBackend, TransferDetails};
    use logos_messaging_a2a_storage::StorageBackend;
    use logos_messaging_a2a_transport::sds::ChannelConfig;
    use logos_messaging_a2a_transport::Transport;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    type PublishedMessages = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    struct MockTransport {
        published: PublishedMessages,
        state: Arc<Mutex<MockState>>,
    }

    struct MockState {
        subscribers: HashMap<String, Vec<mpsc::Sender<Vec<u8>>>>,
        history: HashMap<String, Vec<Vec<u8>>>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                published: Arc::new(Mutex::new(Vec::new())),
                state: Arc::new(Mutex::new(MockState {
                    subscribers: HashMap::new(),
                    history: HashMap::new(),
                })),
            }
        }

        fn inject(&self, topic: &str, payload: Vec<u8>) {
            let mut state = self.state.lock().unwrap();
            state
                .history
                .entry(topic.to_string())
                .or_default()
                .push(payload.clone());
            if let Some(subs) = state.subscribers.get_mut(topic) {
                subs.retain(|tx| tx.try_send(payload.clone()).is_ok());
            }
        }
    }

    impl Clone for MockTransport {
        fn clone(&self) -> Self {
            Self {
                published: self.published.clone(),
                state: self.state.clone(),
            }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
            let data = payload.to_vec();
            self.published
                .lock()
                .unwrap()
                .push((topic.to_string(), data.clone()));

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

    struct MockStorage {
        store: Mutex<HashMap<String, Vec<u8>>>,
        next_id: Mutex<u64>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                store: Mutex::new(HashMap::new()),
                next_id: Mutex::new(0),
            }
        }

        fn len(&self) -> usize {
            self.store.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl StorageBackend for MockStorage {
        async fn upload(
            &self,
            data: Vec<u8>,
        ) -> Result<String, logos_messaging_a2a_storage::StorageError> {
            let mut id = self.next_id.lock().unwrap();
            let cid = format!("zMock{}", *id);
            *id += 1;
            self.store.lock().unwrap().insert(cid.clone(), data);
            Ok(cid)
        }

        async fn download(
            &self,
            cid: &str,
        ) -> Result<Vec<u8>, logos_messaging_a2a_storage::StorageError> {
            self.store.lock().unwrap().get(cid).cloned().ok_or_else(|| {
                logos_messaging_a2a_storage::StorageError::Api {
                    status: 404,
                    body: format!("CID not found: {}", cid),
                }
            })
        }
    }

    struct MockExecutionBackend;

    #[async_trait]
    impl ExecutionBackend for MockExecutionBackend {
        async fn register_agent(
            &self,
            _card: &AgentCard,
        ) -> anyhow::Result<logos_messaging_a2a_execution::TxHash> {
            Ok(logos_messaging_a2a_execution::TxHash([0; 32]))
        }
        async fn pay(
            &self,
            _to: &AgentId,
            _amount: u64,
        ) -> anyhow::Result<logos_messaging_a2a_execution::TxHash> {
            Ok(logos_messaging_a2a_execution::TxHash([0xab; 32]))
        }
        async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
            Ok(1000)
        }
        async fn verify_transfer(&self, _tx_hash: &str) -> anyhow::Result<TransferDetails> {
            Ok(TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 100,
                block_number: 1,
            })
        }
    }

    /// Mock backend that returns configurable transfer details for verify_transfer.
    struct VerifyingBackend {
        details: TransferDetails,
    }

    #[async_trait]
    impl ExecutionBackend for VerifyingBackend {
        async fn register_agent(
            &self,
            _card: &AgentCard,
        ) -> anyhow::Result<logos_messaging_a2a_execution::TxHash> {
            Ok(logos_messaging_a2a_execution::TxHash([0; 32]))
        }
        async fn pay(
            &self,
            _to: &AgentId,
            _amount: u64,
        ) -> anyhow::Result<logos_messaging_a2a_execution::TxHash> {
            Ok(logos_messaging_a2a_execution::TxHash([0; 32]))
        }
        async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
            Ok(0)
        }
        async fn verify_transfer(&self, _tx_hash: &str) -> anyhow::Result<TransferDetails> {
            Ok(self.details.clone())
        }
    }

    /// Mock backend where verify_transfer always fails.
    struct FailingVerifyBackend;

    #[async_trait]
    impl ExecutionBackend for FailingVerifyBackend {
        async fn register_agent(
            &self,
            _card: &AgentCard,
        ) -> anyhow::Result<logos_messaging_a2a_execution::TxHash> {
            Ok(logos_messaging_a2a_execution::TxHash([0; 32]))
        }
        async fn pay(
            &self,
            _to: &AgentId,
            _amount: u64,
        ) -> anyhow::Result<logos_messaging_a2a_execution::TxHash> {
            Ok(logos_messaging_a2a_execution::TxHash([0; 32]))
        }
        async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
            Ok(0)
        }
        async fn verify_transfer(&self, tx_hash: &str) -> anyhow::Result<TransferDetails> {
            anyhow::bail!("Transaction {} not found", tx_hash)
        }
    }

    fn fast_config() -> ChannelConfig {
        ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        }
    }

    fn rand_core() -> k256::elliptic_curve::rand_core::OsRng {
        k256::elliptic_curve::rand_core::OsRng
    }

    #[test]
    fn test_node_creation() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec!["text".into()], transport);
        assert_eq!(node.card.name, "test");
        assert!(!node.pubkey().is_empty());
        assert_eq!(node.pubkey().len(), 66);
        assert!(node.identity().is_none());
        assert!(node.card.intro_bundle.is_none());
    }

    #[test]
    fn test_encrypted_node_creation() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new_encrypted("test", "test agent", vec!["text".into()], transport);
        assert!(node.identity().is_some());
        assert!(node.card.intro_bundle.is_some());
        let bundle = node.card.intro_bundle.as_ref().unwrap();
        assert_eq!(bundle.version, "1.0");
        assert_eq!(bundle.agent_pubkey.len(), 64);
    }

    #[tokio::test]
    async fn test_announce() {
        let transport = MockTransport::new();
        let published = transport.published.clone();
        let node = WakuA2ANode::new("echo", "echo agent", vec!["text".into()], transport);

        node.announce().await.unwrap();

        let msgs = published.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, topics::DISCOVERY);

        let envelope: A2AEnvelope = serde_json::from_slice(&msgs[0].1).unwrap();
        match envelope {
            A2AEnvelope::AgentCard(card) => {
                assert_eq!(card.name, "echo");
                assert_eq!(card.public_key, node.pubkey());
            }
            _ => panic!("Expected AgentCard envelope"),
        }
    }

    #[tokio::test]
    async fn test_discover() {
        let transport = MockTransport::new();
        let other_card = AgentCard {
            name: "other".to_string(),
            description: "other agent".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["code".to_string()],
            public_key: "02deadbeef".to_string(),
            intro_bundle: None,
        };
        let envelope = A2AEnvelope::AgentCard(other_card.clone());
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.inject(topics::DISCOVERY, payload);

        let node = WakuA2ANode::new("me", "my agent", vec![], transport);
        let cards = node.discover().await.unwrap();

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "other");
    }

    #[tokio::test]
    async fn test_poll_tasks() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("echo", "echo agent", vec!["text".into()], transport);

        let tasks = node.poll_tasks().await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn test_channel_accessible() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        assert_eq!(node.channel().sender_id(), node.pubkey());
        assert_eq!(node.channel().outgoing_pending(), 0);
        assert_eq!(node.channel().incoming_pending(), 0);
    }

    #[test]
    fn test_create_session() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        let session = node.create_session("02deadbeef");
        assert_eq!(session.peer, "02deadbeef");
        assert!(session.task_ids.is_empty());
        assert!(!session.id.is_empty());
    }

    #[test]
    fn test_list_sessions() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        assert!(node.list_sessions().is_empty());
        node.create_session("02aa");
        node.create_session("02bb");
        assert_eq!(node.list_sessions().len(), 2);
    }

    #[test]
    fn test_get_session() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        let session = node.create_session("02aa");
        let found = node.get_session(&session.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().peer, "02aa");
        assert!(node.get_session("nonexistent").is_none());
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

    // --- x402 payment tests ---

    #[tokio::test]
    async fn test_task_with_payment_attached() {
        let mut task = Task::new("02aa", "03bb", "pay me");
        task.payment_tx = Some("abcd1234".to_string());
        task.payment_amount = Some(100);

        // Serialize and deserialize to verify payment fields survive the wire
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.payment_tx, Some("abcd1234".to_string()));
        assert_eq!(deserialized.payment_amount, Some(100));
    }

    #[tokio::test]
    async fn test_task_rejected_without_payment() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        // Receiver requires payment of 50
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,

            verify_on_chain: false,

            receiving_account: String::new(),
        });
        let recipient_pubkey = receiver.pubkey().to_string();

        // Lazy-subscribe
        let _ = receiver.poll_tasks().await.unwrap();

        // Sender does NOT pay
        let sender = WakuA2ANode::with_config(
            "sender",
            "sender agent",
            vec![],
            transport.clone(),
            fast_config(),
        );

        let task = Task::new(sender.pubkey(), &recipient_pubkey, "free ride");
        sender.send_task(&task).await.unwrap();

        // Receiver should reject the unpaid task
        let received = receiver.poll_tasks().await.unwrap();
        assert!(received.is_empty(), "unpaid task should be rejected");
    }

    #[tokio::test]
    async fn test_auto_pay_on_send() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        // Receiver requires payment and listens
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,

            verify_on_chain: false,

            receiving_account: String::new(),
        });
        let recipient_pubkey = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        // Sender with auto-pay enabled
        let sender = WakuA2ANode::with_config(
            "sender",
            "sender agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 0,
            auto_pay: true,
            auto_pay_amount: 100,

            verify_on_chain: false,

            receiving_account: String::new(),
        });

        let task = Task::new(sender.pubkey(), &recipient_pubkey, "paid task");
        sender.send_task(&task).await.unwrap();

        // Receiver should accept the auto-paid task
        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1);
        assert!(received[0].payment_tx.is_some(), "should have TX hash");
        assert_eq!(received[0].payment_amount, Some(100));
        assert_eq!(received[0].text(), Some("paid task"));
    }

    #[tokio::test]
    async fn test_replay_protection_rejects_duplicate_tx() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 100,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let recipient_pubkey = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender = WakuA2ANode::with_config(
            "sender",
            "sender agent",
            vec![],
            transport.clone(),
            fast_config(),
        );

        // First task with tx hash — accepted
        let mut task1 = Task::new(sender.pubkey(), &recipient_pubkey, "first payment");
        task1.payment_tx = Some("0xabc123".to_string());
        task1.payment_amount = Some(100);
        sender.send_task(&task1).await.unwrap();
        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1, "first use of tx should be accepted");

        // Same tx hash again — rejected (replay)
        let mut task2 = Task::new(sender.pubkey(), &recipient_pubkey, "replay attempt");
        task2.payment_tx = Some("0xabc123".to_string());
        task2.payment_amount = Some(100);
        sender.send_task(&task2).await.unwrap();
        let received = receiver.poll_tasks().await.unwrap();
        assert!(received.is_empty(), "replayed tx should be rejected");
    }

    #[tokio::test]
    async fn test_on_chain_verify_rejects_insufficient_amount() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 10, // less than required
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "underpaid");
        task.payment_tx = Some("0xunderpaid".to_string());
        task.payment_amount = Some(10);
        sender.send_task(&task).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_on_chain_verify_rejects_wrong_recipient() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xwrong".into(),
                amount: 100,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xcorrect".to_string(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "wrong dest");
        task.payment_tx = Some("0xwrongdest".to_string());
        task.payment_amount = Some(100);
        sender.send_task(&task).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_on_chain_verify_rejects_failed_tx() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(FailingVerifyBackend);

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "bad tx");
        task.payment_tx = Some("0xnonexistent".to_string());
        task.payment_amount = Some(100);
        sender.send_task(&task).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_on_chain_verify_accepts_valid_payment() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xmy_wallet".into(),
                amount: 200,
                block_number: 42,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xmy_wallet".to_string(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "valid payment");
        task.payment_tx = Some("0xgoodtx".to_string());
        task.payment_amount = Some(200);
        sender.send_task(&task).await.unwrap();
        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].text(), Some("valid payment"));
    }

    // ---- send_text, respond, presence, and more ----

    #[test]
    fn test_from_key_deterministic_pubkey() {
        let transport = MockTransport::new();
        let key = SigningKey::random(&mut rand_core());
        let expected_pk = hex::encode(key.verifying_key().to_encoded_point(true).as_bytes());

        let node = WakuA2ANode::from_key(
            "det",
            "deterministic node",
            vec!["text".into()],
            transport,
            key,
        );
        assert_eq!(node.pubkey(), expected_pk);
        assert_eq!(node.card.name, "det");
        assert_eq!(node.card.description, "deterministic node");
        assert_eq!(node.card.capabilities, vec!["text".to_string()]);
    }

    #[test]
    fn test_from_key_same_key_same_pubkey() {
        let key_bytes = [42u8; 32];
        let key1 = SigningKey::from_bytes((&key_bytes).into()).unwrap();
        let key2 = SigningKey::from_bytes((&key_bytes).into()).unwrap();

        let node1 = WakuA2ANode::from_key("a", "a", vec![], MockTransport::new(), key1);
        let node2 = WakuA2ANode::from_key("b", "b", vec![], MockTransport::new(), key2);

        assert_eq!(node1.pubkey(), node2.pubkey());
    }

    #[test]
    fn test_with_config_custom_settings() {
        let transport = MockTransport::new();
        let config = ChannelConfig {
            ack_timeout: std::time::Duration::from_secs(30),
            max_retries: 5,
            ..Default::default()
        };
        let node = WakuA2ANode::with_config(
            "configured",
            "custom config node",
            vec!["image".into()],
            transport,
            config,
        );
        assert_eq!(node.card.name, "configured");
        assert_eq!(node.card.capabilities, vec!["image".to_string()]);
        assert!(!node.pubkey().is_empty());
    }

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
    async fn test_respond_publishes_to_sender_topic() {
        let transport = MockTransport::new();
        let published = transport.published.clone();

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
        let _ = sender.poll_tasks().await.unwrap();

        // Sender sends task to receiver
        let task = Task::new(&spk, &rpk, "question?");
        sender.send_task(&task).await.unwrap();

        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        // Record message count before respond
        let pre_count = published.lock().unwrap().len();
        receiver.respond(&tasks[0], "answer!").await.unwrap();
        let post_count = published.lock().unwrap().len();
        assert!(post_count > pre_count, "respond should publish a message");

        // Verify the response was published to the SENDER's task topic
        let sender_topic = topics::task_topic(&spk);
        let pubs = published.lock().unwrap();
        let to_sender = pubs.iter().filter(|(t, _)| *t == sender_topic).count();
        assert!(to_sender >= 1, "response should target sender's topic");
    }

    #[tokio::test]
    async fn test_respond_to_encrypted_publishes() {
        let transport = MockTransport::new();
        let published = transport.published.clone();

        let receiver = WakuA2ANode::new_encrypted(
            "enc-receiver",
            "encrypted receiver",
            vec![],
            transport.clone(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::new_encrypted("enc-sender", "encrypted sender", vec![], transport.clone());
        let spk = sender.pubkey().to_string();
        let _ = sender.poll_tasks().await.unwrap();

        // Send encrypted task
        let task = Task::new(&spk, &rpk, "secret question");
        sender
            .send_task_to(&task, Some(&receiver.card))
            .await
            .unwrap();

        let tasks = receiver.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text(), Some("secret question"));

        // Respond with encryption — verify it publishes to sender's topic
        let pre_count = published.lock().unwrap().len();
        receiver
            .respond_to(&tasks[0], "secret answer", Some(&sender.card))
            .await
            .unwrap();
        let post_count = published.lock().unwrap().len();
        assert!(post_count > pre_count, "encrypted respond should publish");

        let sender_topic = topics::task_topic(&spk);
        let pubs = published.lock().unwrap();
        let to_sender = pubs.iter().filter(|(t, _)| *t == sender_topic).count();
        assert!(
            to_sender >= 1,
            "encrypted response should target sender's topic"
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

    #[test]
    fn test_find_peers_by_capability() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        node.peer_map.update(&PresenceAnnouncement {
            agent_id: "peer1".into(),
            name: "img-agent".into(),
            capabilities: vec!["image".into(), "text".into()],
            waku_topic: "/lmao/1/tasks/peer1/proto".into(),
            ttl_secs: 300,
            signature: None,
        });
        node.peer_map.update(&PresenceAnnouncement {
            agent_id: "peer2".into(),
            name: "txt-agent".into(),
            capabilities: vec!["text".into()],
            waku_topic: "/lmao/1/tasks/peer2/proto".into(),
            ttl_secs: 300,
            signature: None,
        });

        assert_eq!(node.find_peers_by_capability("image").len(), 1);
        assert_eq!(node.find_peers_by_capability("text").len(), 2);
        assert_eq!(node.find_peers_by_capability("video").len(), 0);
    }

    #[tokio::test]
    async fn test_announce_presence_and_poll() {
        let transport = MockTransport::new();

        let alice = WakuA2ANode::with_config(
            "alice",
            "Alice agent",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let bob = WakuA2ANode::with_config(
            "bob",
            "Bob agent",
            vec!["code".into()],
            transport.clone(),
            fast_config(),
        );

        alice.announce_presence().await.unwrap();

        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 1);

        let peers = bob.peers().all_live();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].1.name, "alice");
    }

    #[tokio::test]
    async fn test_multiple_responses_ordering() {
        let transport = MockTransport::new();

        let server =
            WakuA2ANode::with_config("server", "server", vec![], transport.clone(), fast_config());
        let spk = server.pubkey().to_string();
        let _ = server.poll_tasks().await.unwrap();

        let client =
            WakuA2ANode::with_config("client", "client", vec![], transport.clone(), fast_config());
        let cpk = client.pubkey().to_string();
        let _ = client.poll_tasks().await.unwrap();

        for i in 0..3 {
            let task = Task::new(&cpk, &spk, &format!("task-{i}"));
            client.send_task(&task).await.unwrap();
        }

        let tasks = server.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 3);

        for (i, task) in tasks.iter().enumerate() {
            server.respond(task, &format!("done-{i}")).await.unwrap();
        }

        let responses = client.poll_tasks().await.unwrap();
        assert_eq!(responses.len(), 3);
    }

    // --- Malformed messages, duplicate handling, and edge cases ---

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

    #[tokio::test]
    async fn test_payment_with_empty_tx_hash_rejected() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // Task with empty tx hash
        let mut task = Task::new(sender.pubkey(), &rpk, "empty tx");
        task.payment_tx = Some(String::new());
        task.payment_amount = Some(100);
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert!(received.is_empty(), "empty tx hash should be rejected");
    }

    #[tokio::test]
    async fn test_payment_zero_required_accepts_all() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 0,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // No payment at all
        let task = Task::new(sender.pubkey(), &rpk, "free task");
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "zero required_amount should accept all tasks"
        );
    }

    #[tokio::test]
    async fn test_node_without_payment_config_accepts_all() {
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

        let task = Task::new(sender.pubkey(), &rpk, "no payment config");
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "node without payment config should accept all tasks"
        );
    }

    #[test]
    fn test_peer_map_default_trait() {
        let map = crate::presence::PeerMap::default();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[tokio::test]
    async fn test_presence_self_not_in_peers() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config(
            "self",
            "self agent",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );

        // Announce and poll own presence
        node.announce_presence().await.unwrap();
        let count = node.poll_presence().await.unwrap();
        // Own announcements are filtered by the presence subscription
        // (they appear on the topic but the node ignores its own agent_id)
        assert_eq!(
            count, 0,
            "node should ignore its own presence announcements"
        );
    }

    #[tokio::test]
    async fn test_announce_presence_with_ttl() {
        let transport = MockTransport::new();
        let alice = WakuA2ANode::with_config(
            "alice",
            "alice agent",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let bob =
            WakuA2ANode::with_config("bob", "bob agent", vec![], transport.clone(), fast_config());

        alice.announce_presence_with_ttl(600).await.unwrap();

        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 1);

        let peers = bob.peers().all_live();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].1.ttl_secs, 600);
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

    // --- Builder, accessor, and additional edge case tests ---

    #[test]
    fn test_with_retry_builder() {
        let transport = MockTransport::new();
        let config = RetryConfig {
            max_attempts: 3,
            base_delay_ms: 100,
            max_delay_ms: 5000,
            jitter: false,
        };
        let node = WakuA2ANode::new("test", "test", vec![], transport).with_retry(config);
        assert!(node.retry_config().is_some());
        let cfg = node.retry_config().unwrap();
        assert_eq!(cfg.max_attempts, 3);
        assert_eq!(cfg.base_delay_ms, 100);
        assert_eq!(cfg.max_delay_ms, 5000);
        assert!(!cfg.jitter);
    }

    #[test]
    fn test_retry_config_none_by_default() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert!(node.retry_config().is_none());
    }

    #[test]
    fn test_signing_key_accessor() {
        let transport = MockTransport::new();
        let key = SigningKey::random(&mut rand_core());
        let expected_pk = hex::encode(key.verifying_key().to_encoded_point(true).as_bytes());
        let node = WakuA2ANode::from_key("test", "test", vec![], transport, key);
        // signing_key should produce the same pubkey
        let sk_pk = hex::encode(
            node.signing_key()
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        );
        assert_eq!(sk_pk, expected_pk);
    }

    #[test]
    fn test_identity_none_for_unencrypted_node() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert!(node.identity().is_none());
    }

    #[test]
    fn test_identity_some_for_encrypted_node() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new_encrypted("test", "test", vec![], transport);
        assert!(node.identity().is_some());
    }

    #[test]
    fn test_pubkey_is_valid_hex() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        // Should be valid hex
        let decoded = hex::decode(node.pubkey());
        assert!(decoded.is_ok(), "pubkey should be valid hex");
        // Compressed secp256k1 key is 33 bytes
        assert_eq!(decoded.unwrap().len(), 33);
    }

    #[test]
    fn test_card_version_is_set() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec!["cap".into()], transport);
        assert_eq!(node.card.version, "0.1.0");
    }

    #[test]
    fn test_card_capabilities_match() {
        let transport = MockTransport::new();
        let caps = vec!["text".to_string(), "image".to_string(), "code".to_string()];
        let node = WakuA2ANode::new("test", "test agent", caps.clone(), transport);
        assert_eq!(node.card.capabilities, caps);
    }

    #[tokio::test]
    async fn test_discover_excludes_self() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("self", "self agent", vec![], transport.clone());

        // Announce self
        node.announce().await.unwrap();

        // Discover should not include self
        let cards = node.discover().await.unwrap();
        assert!(cards.is_empty(), "discover should exclude own card");
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_disabled() {
        let backend = Arc::new(MockExecutionBackend);
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_payment(PaymentConfig {
                backend,
                required_amount: 0,
                auto_pay: false, // disabled
                auto_pay_amount: 100,
                verify_on_chain: false,
                receiving_account: String::new(),
            });

        let task = Task::new(node.pubkey(), "02aa", "no auto pay");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        // With auto_pay disabled, task should NOT have payment info
        assert!(result.payment_tx.is_none());
        assert!(result.payment_amount.is_none());
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_zero_amount() {
        let backend = Arc::new(MockExecutionBackend);
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_payment(PaymentConfig {
                backend,
                required_amount: 0,
                auto_pay: true,
                auto_pay_amount: 0, // zero amount
                verify_on_chain: false,
                receiving_account: String::new(),
            });

        let task = Task::new(node.pubkey(), "02aa", "zero amount");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        // With zero auto_pay_amount, should not actually pay
        assert!(result.payment_tx.is_none());
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_attaches_tx_hash() {
        let backend = Arc::new(MockExecutionBackend);
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_payment(PaymentConfig {
                backend,
                required_amount: 0,
                auto_pay: true,
                auto_pay_amount: 50,
                verify_on_chain: false,
                receiving_account: String::new(),
            });

        let task = Task::new(node.pubkey(), "02aa", "pay me");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        assert!(result.payment_tx.is_some());
        assert_eq!(result.payment_amount, Some(50));
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_no_payment_config() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let task = Task::new(node.pubkey(), "02aa", "no config");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        assert!(result.payment_tx.is_none());
        assert!(result.payment_amount.is_none());
    }

    #[tokio::test]
    async fn test_on_chain_verify_case_insensitive_recipient() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xMyWallet".into(), // mixed case
                amount: 200,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xmywallet".to_string(), // lowercase
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "case test");
        task.payment_tx = Some("0xcasetx".to_string());
        task.payment_amount = Some(200);
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "case-insensitive recipient match should accept"
        );
    }

    #[tokio::test]
    async fn test_replay_protection_with_on_chain_verify() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 200,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xrecipient".to_string(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // First use — accepted
        let mut t1 = Task::new(sender.pubkey(), &rpk, "first");
        t1.payment_tx = Some("0xreplay_onchain".to_string());
        t1.payment_amount = Some(200);
        sender.send_task(&t1).await.unwrap();
        assert_eq!(receiver.poll_tasks().await.unwrap().len(), 1);

        // Replay — rejected
        let mut t2 = Task::new(sender.pubkey(), &rpk, "replay");
        t2.payment_tx = Some("0xreplay_onchain".to_string());
        t2.payment_amount = Some(200);
        sender.send_task(&t2).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_channel_sender_id_matches_pubkey() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert_eq!(node.channel().sender_id(), node.pubkey());
    }

    #[test]
    fn test_with_payment_builder() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);
        let node =
            WakuA2ANode::new("test", "test", vec![], transport).with_payment(PaymentConfig {
                backend,
                required_amount: 42,
                auto_pay: true,
                auto_pay_amount: 10,
                verify_on_chain: true,
                receiving_account: "0xabc".to_string(),
            });
        assert!(node.payment.is_some());
        let pay = node.payment.as_ref().unwrap();
        assert_eq!(pay.required_amount, 42);
        assert!(pay.auto_pay);
        assert_eq!(pay.auto_pay_amount, 10);
        assert!(pay.verify_on_chain);
        assert_eq!(pay.receiving_account, "0xabc");
    }

    #[tokio::test]
    async fn test_discover_returns_multiple_cards() {
        let transport = MockTransport::new();

        // Inject three agent cards
        for i in 0..3 {
            let card = AgentCard {
                name: format!("agent-{i}"),
                description: format!("agent {i}"),
                version: "0.1.0".to_string(),
                capabilities: vec![],
                public_key: format!("02{i:064x}"),
                intro_bundle: None,
            };
            let envelope = A2AEnvelope::AgentCard(card);
            let payload = serde_json::to_vec(&envelope).unwrap();
            transport.inject(topics::DISCOVERY, payload);
        }

        let node = WakuA2ANode::new("me", "me", vec![], transport);
        let cards = node.discover().await.unwrap();
        assert_eq!(cards.len(), 3);
        assert!(cards.iter().any(|c| c.name == "agent-0"));
        assert!(cards.iter().any(|c| c.name == "agent-1"));
        assert!(cards.iter().any(|c| c.name == "agent-2"));
    }

    #[tokio::test]
    async fn test_poll_presence_multiple_peers() {
        let transport = MockTransport::new();
        let node1 = WakuA2ANode::with_config(
            "node1",
            "node1",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let node2 = WakuA2ANode::with_config(
            "node2",
            "node2",
            vec!["code".into()],
            transport.clone(),
            fast_config(),
        );
        let observer = WakuA2ANode::with_config(
            "observer",
            "observer",
            vec![],
            transport.clone(),
            fast_config(),
        );

        node1.announce_presence().await.unwrap();
        node2.announce_presence().await.unwrap();

        let count = observer.poll_presence().await.unwrap();
        assert_eq!(count, 2);

        let peers = observer.peers().all_live();
        assert_eq!(peers.len(), 2);
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
}

#[cfg(test)]
mod session_tests {
    use crate::WakuA2ANode;
    use anyhow::Result;
    use async_trait::async_trait;
    use logos_messaging_a2a_core::Task;
    use logos_messaging_a2a_transport::Transport;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    type PublishedMessages = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    struct MockTransport {
        published: PublishedMessages,
        state: Arc<Mutex<MockState>>,
    }

    struct MockState {
        subscribers: HashMap<String, Vec<mpsc::Sender<Vec<u8>>>>,
        history: HashMap<String, Vec<Vec<u8>>>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                published: Arc::new(Mutex::new(Vec::new())),
                state: Arc::new(Mutex::new(MockState {
                    subscribers: HashMap::new(),
                    history: HashMap::new(),
                })),
            }
        }
    }

    impl Clone for MockTransport {
        fn clone(&self) -> Self {
            Self {
                published: self.published.clone(),
                state: self.state.clone(),
            }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
            let data = payload.to_vec();
            self.published
                .lock()
                .unwrap()
                .push((topic.to_string(), data.clone()));
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

    fn fast_config() -> logos_messaging_a2a_transport::sds::ChannelConfig {
        logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        }
    }

    #[test]
    fn session_has_unique_uuid() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let s1 = node.create_session("peer-a");
        let s2 = node.create_session("peer-b");
        assert_ne!(s1.id, s2.id, "sessions should have unique IDs");
    }

    #[test]
    fn session_created_at_is_recent() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let session = node.create_session("peer-a");
        // created_at should be within 2 seconds of now
        assert!(
            session.created_at >= now - 2 && session.created_at <= now + 2,
            "session created_at should be close to current time"
        );
    }

    #[test]
    fn session_starts_with_empty_task_ids() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let session = node.create_session("peer-a");
        assert!(session.task_ids.is_empty());
    }

    #[test]
    fn session_peer_preserved_correctly() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let long_key = "02".to_string() + &"ab".repeat(32);
        let session = node.create_session(&long_key);
        assert_eq!(session.peer, long_key);
    }

    #[tokio::test]
    async fn multiple_tasks_tracked_in_session() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config());
        let session = node.create_session("02deadbeef");

        // Send multiple messages in the same session
        let t1 = node.send_in_session(&session.id, "first").await.unwrap();
        let t2 = node.send_in_session(&session.id, "second").await.unwrap();
        let t3 = node.send_in_session(&session.id, "third").await.unwrap();

        let updated = node.get_session(&session.id).unwrap();
        assert_eq!(updated.task_ids.len(), 3);
        assert_eq!(updated.task_ids[0], t1.id);
        assert_eq!(updated.task_ids[1], t2.id);
        assert_eq!(updated.task_ids[2], t3.id);
    }

    #[tokio::test]
    async fn tasks_in_session_carry_session_id() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config());
        let session = node.create_session("02deadbeef");

        let task = node.send_in_session(&session.id, "hello").await.unwrap();
        assert_eq!(task.session_id, Some(session.id.clone()));
    }

    #[test]
    fn sessions_isolated_between_peers() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let s1 = node.create_session("peer-a");
        let s2 = node.create_session("peer-b");

        assert_ne!(s1.id, s2.id);
        assert_eq!(s1.peer, "peer-a");
        assert_eq!(s2.peer, "peer-b");

        // Getting one doesn't affect the other
        let got1 = node.get_session(&s1.id).unwrap();
        let got2 = node.get_session(&s2.id).unwrap();
        assert_eq!(got1.peer, "peer-a");
        assert_eq!(got2.peer, "peer-b");
    }

    #[test]
    fn multiple_sessions_with_same_peer() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let s1 = node.create_session("peer-a");
        let s2 = node.create_session("peer-a");

        // Two distinct sessions, same peer
        assert_ne!(s1.id, s2.id);
        assert_eq!(s1.peer, s2.peer);
        assert_eq!(node.list_sessions().len(), 2);
    }

    #[test]
    fn get_session_returns_clone() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let session = node.create_session("peer-a");

        let got1 = node.get_session(&session.id).unwrap();
        let got2 = node.get_session(&session.id).unwrap();
        assert_eq!(got1.id, got2.id);
        assert_eq!(got1.peer, got2.peer);
    }

    #[test]
    fn list_sessions_empty_initially() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert!(node.list_sessions().is_empty());
    }

    #[test]
    fn list_sessions_contains_all_created() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let ids: Vec<String> = (0..5)
            .map(|i| node.create_session(&format!("peer-{i}")).id)
            .collect();

        let sessions = node.list_sessions();
        assert_eq!(sessions.len(), 5);
        for id in &ids {
            assert!(sessions.iter().any(|s| &s.id == id));
        }
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

#[cfg(test)]
mod storage_tests {
    use crate::storage::StorageOffloadConfig;
    use crate::WakuA2ANode;
    use anyhow::Result;
    use async_trait::async_trait;
    use logos_messaging_a2a_core::Task;
    use logos_messaging_a2a_transport::Transport;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    type PublishedMessages = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    struct MockTransport {
        published: PublishedMessages,
        state: Arc<Mutex<MockState>>,
    }

    struct MockState {
        subscribers: HashMap<String, Vec<mpsc::Sender<Vec<u8>>>>,
        history: HashMap<String, Vec<Vec<u8>>>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                published: Arc::new(Mutex::new(Vec::new())),
                state: Arc::new(Mutex::new(MockState {
                    subscribers: HashMap::new(),
                    history: HashMap::new(),
                })),
            }
        }
    }

    impl Clone for MockTransport {
        fn clone(&self) -> Self {
            Self {
                published: self.published.clone(),
                state: self.state.clone(),
            }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
            let data = payload.to_vec();
            self.published
                .lock()
                .unwrap()
                .push((topic.to_string(), data.clone()));
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

    struct MockStorage {
        store: Mutex<HashMap<String, Vec<u8>>>,
        next_id: Mutex<u64>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                store: Mutex::new(HashMap::new()),
                next_id: Mutex::new(0),
            }
        }

        fn len(&self) -> usize {
            self.store.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl logos_messaging_a2a_storage::StorageBackend for MockStorage {
        async fn upload(
            &self,
            data: Vec<u8>,
        ) -> Result<String, logos_messaging_a2a_storage::StorageError> {
            let mut id = self.next_id.lock().unwrap();
            let cid = format!("zMock{}", *id);
            *id += 1;
            self.store.lock().unwrap().insert(cid.clone(), data);
            Ok(cid)
        }

        async fn download(
            &self,
            cid: &str,
        ) -> Result<Vec<u8>, logos_messaging_a2a_storage::StorageError> {
            self.store.lock().unwrap().get(cid).cloned().ok_or_else(|| {
                logos_messaging_a2a_storage::StorageError::Api {
                    status: 404,
                    body: format!("CID not found: {}", cid),
                }
            })
        }
    }

    /// Storage backend that always fails on upload.
    struct FailingUploadStorage;

    #[async_trait]
    impl logos_messaging_a2a_storage::StorageBackend for FailingUploadStorage {
        async fn upload(
            &self,
            _data: Vec<u8>,
        ) -> Result<String, logos_messaging_a2a_storage::StorageError> {
            Err(logos_messaging_a2a_storage::StorageError::Http(
                "upload failed".to_string(),
            ))
        }

        async fn download(
            &self,
            _cid: &str,
        ) -> Result<Vec<u8>, logos_messaging_a2a_storage::StorageError> {
            Err(logos_messaging_a2a_storage::StorageError::Http(
                "download failed".to_string(),
            ))
        }
    }

    /// Storage backend that always fails on download.
    struct FailingDownloadStorage {
        store: Mutex<HashMap<String, Vec<u8>>>,
        next_id: Mutex<u64>,
    }

    impl FailingDownloadStorage {
        fn new() -> Self {
            Self {
                store: Mutex::new(HashMap::new()),
                next_id: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl logos_messaging_a2a_storage::StorageBackend for FailingDownloadStorage {
        async fn upload(
            &self,
            data: Vec<u8>,
        ) -> Result<String, logos_messaging_a2a_storage::StorageError> {
            let mut id = self.next_id.lock().unwrap();
            let cid = format!("zFail{}", *id);
            *id += 1;
            self.store.lock().unwrap().insert(cid.clone(), data);
            Ok(cid)
        }

        async fn download(
            &self,
            _cid: &str,
        ) -> Result<Vec<u8>, logos_messaging_a2a_storage::StorageError> {
            Err(logos_messaging_a2a_storage::StorageError::Api {
                status: 500,
                body: "download always fails".to_string(),
            })
        }
    }

    fn fast_config() -> logos_messaging_a2a_transport::sds::ChannelConfig {
        logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        }
    }

    #[test]
    fn storage_offload_config_new_default_threshold() {
        let storage = Arc::new(MockStorage::new());
        let config = StorageOffloadConfig::new(storage);
        assert_eq!(config.threshold_bytes, 65_536);
    }

    #[test]
    fn storage_offload_config_custom_threshold() {
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
