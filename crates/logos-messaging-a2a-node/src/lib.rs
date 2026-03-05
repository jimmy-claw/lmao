use anyhow::{Context, Result};
use k256::ecdsa::SigningKey;
pub use logos_messaging_a2a_core::Task as TaskType;
use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard, Message, Task};
use logos_messaging_a2a_storage::StorageBackend;
use logos_messaging_a2a_crypto::{AgentIdentity, IntroBundle};
use logos_messaging_a2a_transport::sds::{ChannelConfig, MessageChannel};
use logos_messaging_a2a_transport::Transport;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// A multi-turn conversation session between two agents.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub peer: String,
    pub task_ids: Vec<String>,
    pub created_at: u64,
}

impl Session {
    fn new(peer: &str) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id,
            peer: peer.to_string(),
            task_ids: Vec::new(),
            created_at,
        }
    }
}

/// Configuration for offloading large payloads to Logos Storage.
///
/// When a serialized message envelope exceeds `threshold_bytes`, the payload
/// is uploaded to storage and only the CID is sent over the Waku network.
/// The receiver automatically fetches the full payload by CID.
pub struct StorageOffloadConfig {
    /// Storage backend for uploading/downloading payloads.
    pub backend: Arc<dyn StorageBackend>,
    /// Payload size threshold in bytes. Payloads larger than this are offloaded.
    /// Default: 65 536 (64 KB).
    pub threshold_bytes: usize,
}

impl StorageOffloadConfig {
    /// Create a new config with the given backend and default threshold (64 KB).
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self {
            backend,
            threshold_bytes: 65_536,
        }
    }

    /// Create with a custom threshold.
    pub fn with_threshold(backend: Arc<dyn StorageBackend>, threshold_bytes: usize) -> Self {
        Self {
            backend,
            threshold_bytes,
        }
    }
}

/// A2A node: announce, discover, send/receive tasks over Waku.
///
/// Uses SDS MessageChannel for reliable, causally-ordered delivery with
/// bloom filter deduplication and implicit ACK via remote bloom filters.
pub struct WakuA2ANode<T: Transport> {
    pub card: AgentCard,
    channel: MessageChannel<T>,
    signing_key: SigningKey,
    /// Optional X25519 identity for encrypted sessions.
    identity: Option<AgentIdentity>,
    /// Persistent subscription to this node's task topic (lazy-initialized).
    task_rx: tokio::sync::Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    /// Active conversation sessions.
    sessions: std::sync::Mutex<HashMap<String, Session>>,
    /// Optional storage offload for large payloads.
    storage_offload: Option<StorageOffloadConfig>,
}

impl<T: Transport> WakuA2ANode<T> {
    /// Create a new node with a random keypair (no encryption).
    pub fn new(name: &str, description: &str, capabilities: Vec<String>, transport: T) -> Self {
        let signing_key = SigningKey::random(&mut rand_core());
        let public_key = hex::encode(
            signing_key
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        );

        let card = AgentCard {
            name: name.to_string(),
            description: description.to_string(),
            version: "0.1.0".to_string(),
            capabilities,
            public_key: public_key.clone(),
            intro_bundle: None,
        };

        Self {
            card,
            channel: MessageChannel::new(
                format!("node-{}", &public_key[..8]),
                public_key,
                transport,
            ),
            signing_key,
            identity: None,
            task_rx: tokio::sync::Mutex::new(None),
            sessions: std::sync::Mutex::new(HashMap::new()),
            storage_offload: None,
        }
    }

    /// Create a new node with encryption enabled.
    pub fn new_encrypted(
        name: &str,
        description: &str,
        capabilities: Vec<String>,
        transport: T,
    ) -> Self {
        let signing_key = SigningKey::random(&mut rand_core());
        let public_key = hex::encode(
            signing_key
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        );

        let identity = AgentIdentity::generate();
        let intro_bundle = IntroBundle::new(&identity.public_key_hex());

        let card = AgentCard {
            name: name.to_string(),
            description: description.to_string(),
            version: "0.1.0".to_string(),
            capabilities,
            public_key: public_key.clone(),
            intro_bundle: Some(intro_bundle),
        };

        Self {
            card,
            channel: MessageChannel::new(
                format!("node-{}", &public_key[..8]),
                public_key,
                transport,
            ),
            signing_key,
            identity: Some(identity),
            task_rx: tokio::sync::Mutex::new(None),
            sessions: std::sync::Mutex::new(HashMap::new()),
            storage_offload: None,
        }
    }

    /// Create a node from an existing signing key (no encryption).
    pub fn from_key(
        name: &str,
        description: &str,
        capabilities: Vec<String>,
        transport: T,
        signing_key: SigningKey,
    ) -> Self {
        let public_key = hex::encode(
            signing_key
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        );

        let card = AgentCard {
            name: name.to_string(),
            description: description.to_string(),
            version: "0.1.0".to_string(),
            capabilities,
            public_key: public_key.clone(),
            intro_bundle: None,
        };

        Self {
            card,
            channel: MessageChannel::new(
                format!("node-{}", &public_key[..8]),
                public_key,
                transport,
            ),
            signing_key,
            identity: None,
            task_rx: tokio::sync::Mutex::new(None),
            sessions: std::sync::Mutex::new(HashMap::new()),
            storage_offload: None,
        }
    }

    /// Create a node with custom SDS channel configuration.
    pub fn with_config(
        name: &str,
        description: &str,
        capabilities: Vec<String>,
        transport: T,
        config: ChannelConfig,
    ) -> Self {
        let signing_key = SigningKey::random(&mut rand_core());
        let public_key = hex::encode(
            signing_key
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        );

        let card = AgentCard {
            name: name.to_string(),
            description: description.to_string(),
            version: "0.1.0".to_string(),
            capabilities,
            public_key: public_key.clone(),
            intro_bundle: None,
        };

        Self {
            card,
            channel: MessageChannel::with_config(
                format!("node-{}", &public_key[..8]),
                public_key,
                transport,
                config,
            ),
            signing_key,
            identity: None,
            task_rx: tokio::sync::Mutex::new(None),
            sessions: std::sync::Mutex::new(HashMap::new()),
            storage_offload: None,
        }
    }

    /// Enable CID-based offloading of large payloads to Logos Storage.
    ///
    /// When configured, payloads exceeding the threshold are automatically
    /// uploaded to storage. Only the CID is sent over Waku. Receivers with
    /// the same config auto-fetch the full payload by CID.
    pub fn with_storage_offload(mut self, config: StorageOffloadConfig) -> Self {
        self.storage_offload = Some(config);
        self
    }

    /// Get this agent's public key hex string.
    pub fn pubkey(&self) -> &str {
        &self.card.public_key
    }

    /// Get the signing key (for testing or advanced use).
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    /// Get the encryption identity (if encryption is enabled).
    pub fn identity(&self) -> Option<&AgentIdentity> {
        self.identity.as_ref()
    }

    /// Access the underlying SDS MessageChannel.
    pub fn channel(&self) -> &MessageChannel<T> {
        &self.channel
    }

    /// Broadcast this agent's card on the discovery topic.
    ///
    /// Discovery uses raw A2AEnvelope (not SDS-wrapped) since it's a
    /// broadcast to unknown peers who may not speak SDS yet.
    pub async fn announce(&self) -> Result<()> {
        let envelope = A2AEnvelope::AgentCard(self.card.clone());
        let payload = serde_json::to_vec(&envelope).context("Failed to serialize AgentCard")?;
        self.channel
            .transport()
            .publish(topics::DISCOVERY, &payload)
            .await
            .context("Failed to announce AgentCard")?;
        eprintln!("[node] Announced: {} ({})", self.card.name, self.pubkey());
        Ok(())
    }

    /// Discover agents by subscribing to the discovery topic and draining messages.
    pub async fn discover(&self) -> Result<Vec<AgentCard>> {
        let mut rx = self
            .channel
            .transport()
            .subscribe(topics::DISCOVERY)
            .await?;

        let mut cards = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Ok(A2AEnvelope::AgentCard(card)) = serde_json::from_slice(&msg) {
                if card.public_key != self.card.public_key {
                    cards.push(card);
                }
            }
        }

        let _ = self
            .channel
            .transport()
            .unsubscribe(topics::DISCOVERY)
            .await;
        Ok(cards)
    }

    /// Send a task to another agent. Uses SDS reliable delivery with
    /// causal ordering, bloom filter, and retransmission.
    pub async fn send_task(&self, task: &Task) -> Result<bool> {
        self.send_task_to(task, None).await
    }

    /// Send a task, optionally encrypting if recipient has an intro bundle.
    pub async fn send_task_to(
        &self,
        task: &Task,
        recipient_card: Option<&AgentCard>,
    ) -> Result<bool> {
        let topic = topics::task_topic(&task.to);
        let payload = self.prepare_payload(task, recipient_card).await?;

        // Use SDS reliable delivery — the SDS message_id (SHA256 of payload)
        // is used for ACK routing, not the task UUID.
        let (_msg, acked) = self
            .channel
            .send_reliable(&topic, &payload)
            .await
            .context("SDS publish failed")?;

        if acked {
            eprintln!("[node] Task {} sent and ACKed", task.id);
        } else {
            eprintln!("[node] Task {} sent but no ACK received", task.id);
        }
        Ok(acked)
    }

    /// Create a new conversation session with a peer.
    pub fn create_session(&self, peer_pubkey: &str) -> Session {
        let session = Session::new(peer_pubkey);
        let id = session.id.clone();
        self.sessions
            .lock()
            .unwrap()
            .insert(id.clone(), session.clone());
        session
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: &str) -> Option<Session> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    /// List all active sessions.
    pub fn list_sessions(&self) -> Vec<Session> {
        self.sessions.lock().unwrap().values().cloned().collect()
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
                            eprintln!("[node] Failed to decrypt task: {}", e);
                            Ok(None)
                        }
                    }
                } else {
                    eprintln!("[node] Received encrypted task but no identity configured");
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
            eprintln!(
                "[node] Task has payload_cid but no storage backend configured"
            );
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

        eprintln!("[node] Responded to task {}", task.id);
        Ok(())
    }

    /// Create a task and send it.
    pub async fn send_text(&self, to: &str, text: &str) -> Result<Task> {
        let task = Task::new(self.pubkey(), to, text);
        self.send_task(&task).await?;
        Ok(task)
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

/// Platform-appropriate RNG.
fn rand_core() -> k256::elliptic_curve::rand_core::OsRng {
    k256::elliptic_curve::rand_core::OsRng
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MockTransport {
        published: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
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
            self.store
                .lock()
                .unwrap()
                .get(cid)
                .cloned()
                .ok_or_else(|| logos_messaging_a2a_storage::StorageError::Api {
                    status: 404,
                    body: format!("CID not found: {}", cid),
                })
        }
    }

    fn fast_config() -> ChannelConfig {
        ChannelConfig {
            ack_timeout: std::time::Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_small_payload_inline() {
        let transport = MockTransport::new();
        let published = transport.published.clone();
        let storage = Arc::new(MockStorage::new());

        let node = WakuA2ANode::with_config(
            "test",
            "test agent",
            vec![],
            transport,
            fast_config(),
        )
        .with_storage_offload(StorageOffloadConfig::with_threshold(storage.clone(), 65_536));

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
        let node = WakuA2ANode::with_config(
            "test",
            "test agent",
            vec![],
            transport,
            fast_config(),
        )
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
}
