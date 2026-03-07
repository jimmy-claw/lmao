pub mod presence;

use anyhow::{Context, Result};
use k256::ecdsa::SigningKey;
use logos_messaging_a2a_core::registry::AgentRegistry;
pub use logos_messaging_a2a_core::Task as TaskType;
use logos_messaging_a2a_core::{
    topics, A2AEnvelope, AgentCard, Message, PresenceAnnouncement, Task,
};
use logos_messaging_a2a_crypto::{AgentIdentity, IntroBundle};
use logos_messaging_a2a_execution::{AgentId, ExecutionBackend};
use logos_messaging_a2a_storage::StorageBackend;
use logos_messaging_a2a_transport::sds::{ChannelConfig, MessageChannel};
use logos_messaging_a2a_transport::Transport;
use std::collections::{HashMap, HashSet};
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

/// Configuration for x402-style payment flow via [`ExecutionBackend`].
///
/// When configured on a node:
/// - **Sending**: if `auto_pay` is true, `backend.pay()` is called before
///   sending and the TX hash is attached to the outgoing task.
/// - **Receiving**: if `required_amount > 0`, incoming tasks without a valid
///   `payment_tx` (or with insufficient `payment_amount`) are rejected.
pub struct PaymentConfig {
    /// Execution backend used for `pay()` / `balance()` / `verify_transfer()` calls.
    pub backend: Arc<dyn ExecutionBackend>,
    /// Minimum payment required to accept an incoming task. 0 = no requirement.
    pub required_amount: u64,
    /// Automatically pay when sending tasks.
    pub auto_pay: bool,
    /// Amount to auto-pay per outgoing task (only used when `auto_pay` is true).
    pub auto_pay_amount: u64,
    /// When true, verify payment tx hashes on-chain via `backend.verify_transfer()`.
    /// When false, only check that `payment_tx` and `payment_amount` are present.
    pub verify_on_chain: bool,
    /// Expected recipient address for on-chain verification (lowercase hex with 0x).
    /// If empty, recipient is not checked (only amount is validated).
    pub receiving_account: String,
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
    /// Optional x402-style payment configuration.
    payment: Option<PaymentConfig>,
    /// Set of already-seen payment tx hashes to prevent replay attacks.
    seen_tx_hashes: std::sync::Mutex<HashSet<String>>,
    /// Live peer map built from presence announcements.
    peer_map: presence::PeerMap,
    /// Persistent subscription to the presence topic (lazy-initialized).
    presence_rx: tokio::sync::Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    /// Optional persistent agent registry (e.g. LEZ on-chain).
    registry: Option<Arc<dyn AgentRegistry>>,
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
            payment: None,
            seen_tx_hashes: std::sync::Mutex::new(HashSet::new()),
            peer_map: presence::PeerMap::new(),
            presence_rx: tokio::sync::Mutex::new(None),
            registry: None,
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
            payment: None,
            seen_tx_hashes: std::sync::Mutex::new(HashSet::new()),
            peer_map: presence::PeerMap::new(),
            presence_rx: tokio::sync::Mutex::new(None),
            registry: None,
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
            payment: None,
            seen_tx_hashes: std::sync::Mutex::new(HashSet::new()),
            peer_map: presence::PeerMap::new(),
            presence_rx: tokio::sync::Mutex::new(None),
            registry: None,
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
            payment: None,
            seen_tx_hashes: std::sync::Mutex::new(HashSet::new()),
            peer_map: presence::PeerMap::new(),
            presence_rx: tokio::sync::Mutex::new(None),
            registry: None,
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

    /// Enable x402-style payment flow via an [`ExecutionBackend`].
    ///
    /// When configured, outgoing tasks can auto-pay and incoming tasks can
    /// require payment proof before processing.
    pub fn with_payment(mut self, config: PaymentConfig) -> Self {
        self.payment = Some(config);
        self
    }

    /// Attach a persistent agent registry for on-chain discovery.
    ///
    /// When set, [`discover_all`](Self::discover_all) merges results from both
    /// Waku presence and the registry. The node can also
    /// [`register`](Self::register_in_registry) itself for permanent discovery.
    pub fn with_registry(mut self, registry: Arc<dyn AgentRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Default presence TTL in seconds (5 minutes).
    const DEFAULT_PRESENCE_TTL: u64 = 300;

    /// Broadcast a presence announcement on the well-known presence topic.
    ///
    /// Other agents subscribed to the presence topic will update their
    /// `PeerMap` with this node's identity and capabilities.
    pub async fn announce_presence(&self) -> Result<()> {
        self.announce_presence_with_ttl(Self::DEFAULT_PRESENCE_TTL)
            .await
    }

    /// Broadcast a presence announcement with a custom TTL.
    pub async fn announce_presence_with_ttl(&self, ttl_secs: u64) -> Result<()> {
        let announcement = PresenceAnnouncement {
            agent_id: self.pubkey().to_string(),
            name: self.card.name.clone(),
            capabilities: self.card.capabilities.clone(),
            waku_topic: topics::task_topic(self.pubkey()),
            ttl_secs,
            signature: None, // TODO: sign with secp256k1 key
        };
        let envelope = A2AEnvelope::Presence(announcement);
        let payload =
            serde_json::to_vec(&envelope).context("Failed to serialize presence announcement")?;
        self.channel
            .transport()
            .publish(topics::PRESENCE, &payload)
            .await
            .context("Failed to publish presence announcement")?;
        eprintln!(
            "[node] Presence announced: {} (TTL={}s)",
            self.card.name, ttl_secs
        );
        Ok(())
    }

    /// Poll the presence topic for new announcements and update the peer map.
    ///
    /// Call this periodically (or before routing a task) to keep the peer
    /// map fresh. Ignores announcements from this node itself.
    pub async fn poll_presence(&self) -> Result<usize> {
        let mut presence_rx = self.presence_rx.lock().await;
        if presence_rx.is_none() {
            *presence_rx = Some(self.channel.transport().subscribe(topics::PRESENCE).await?);
        }
        let rx = presence_rx.as_mut().unwrap();

        let mut count = 0;
        while let Ok(msg) = rx.try_recv() {
            if let Ok(A2AEnvelope::Presence(ann)) = serde_json::from_slice::<A2AEnvelope>(&msg) {
                if ann.agent_id != self.pubkey() {
                    self.peer_map.update(&ann);
                    eprintln!(
                        "[node] Presence received: {} ({}) caps={:?}",
                        ann.name,
                        &ann.agent_id[..8.min(ann.agent_id.len())],
                        ann.capabilities
                    );
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Get a reference to the live peer map.
    pub fn peers(&self) -> &presence::PeerMap {
        &self.peer_map
    }

    /// Find peers by capability from the live peer map.
    pub fn find_peers_by_capability(&self, capability: &str) -> Vec<(String, presence::PeerInfo)> {
        self.peer_map.find_by_capability(capability)
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

    /// Register this node's AgentCard in the persistent registry.
    ///
    /// Returns an error if no registry is configured or if the agent
    /// is already registered (use [`update_registry`](Self::update_registry)
    /// to update an existing registration).
    pub async fn register_in_registry(&self) -> Result<()> {
        let registry = self.registry.as_ref().context("no registry configured")?;
        registry
            .register(self.card.clone())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Update this node's AgentCard in the persistent registry.
    pub async fn update_registry(&self) -> Result<()> {
        let registry = self.registry.as_ref().context("no registry configured")?;
        registry
            .update(self.card.clone())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Remove this node from the persistent registry.
    pub async fn deregister_from_registry(&self) -> Result<()> {
        let registry = self.registry.as_ref().context("no registry configured")?;
        registry
            .deregister(&self.card.public_key)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Discover agents from all sources: Waku ephemeral discovery + persistent registry.
    ///
    /// Deduplicates by public key, preferring the registry version when both exist
    /// (since on-chain data is the source of truth).
    pub async fn discover_all(&self) -> Result<Vec<AgentCard>> {
        let mut by_key: HashMap<String, AgentCard> = HashMap::new();

        // Waku ephemeral discovery first
        let waku_cards = self.discover().await?;
        for card in waku_cards {
            by_key.insert(card.public_key.clone(), card);
        }

        // Registry overwrites (source of truth)
        if let Some(ref registry) = self.registry {
            if let Ok(reg_cards) = registry.list().await {
                for card in reg_cards {
                    if card.public_key != self.card.public_key {
                        by_key.insert(card.public_key.clone(), card);
                    }
                }
            }
        }

        Ok(by_key.into_values().collect())
    }

    /// Send a task to another agent. Uses SDS reliable delivery with
    /// causal ordering, bloom filter, and retransmission.
    pub async fn send_task(&self, task: &Task) -> Result<bool> {
        self.send_task_to(task, None).await
    }

    /// Send a task, optionally encrypting if recipient has an intro bundle.
    ///
    /// When a [`PaymentConfig`] with `auto_pay = true` is set, the node
    /// calls `backend.pay()` before sending and attaches the TX hash to
    /// the task envelope.
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
            eprintln!("[node] Task has payload_cid but no storage backend configured");
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
                eprintln!(
                    "[node] Rejecting task {} — no payment tx hash provided (need {} tokens)",
                    task.id, pay_cfg.required_amount
                );
                return false;
            }
        };

        // Check for replayed tx hashes
        {
            let seen = self.seen_tx_hashes.lock().unwrap();
            if seen.contains(&tx_hash) {
                eprintln!(
                    "[node] Rejecting task {} — replayed payment tx {}",
                    task.id, tx_hash
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
                    eprintln!(
                        "[node] Rejecting task {} — insufficient payment (need {}, got {:?})",
                        task.id, pay_cfg.required_amount, task.payment_amount
                    );
                    false
                }
            }
        } else {
            // On-chain verification
            match pay_cfg.backend.verify_transfer(&tx_hash).await {
                Ok(details) => {
                    if details.amount < pay_cfg.required_amount {
                        eprintln!(
                            "[node] Rejecting task {} — on-chain amount {} < required {}",
                            task.id, details.amount, pay_cfg.required_amount
                        );
                        return false;
                    }
                    if !pay_cfg.receiving_account.is_empty()
                        && details.to.to_lowercase() != pay_cfg.receiving_account.to_lowercase()
                    {
                        eprintln!(
                            "[node] Rejecting task {} — payment sent to {} but expected {}",
                            task.id, details.to, pay_cfg.receiving_account
                        );
                        return false;
                    }
                    // Mark as seen to prevent replay
                    self.seen_tx_hashes.lock().unwrap().insert(tx_hash);
                    true
                }
                Err(e) => {
                    eprintln!(
                        "[node] Rejecting task {} — on-chain verification failed: {}",
                        task.id, e
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

/// Platform-appropriate RNG.
fn rand_core() -> k256::elliptic_curve::rand_core::OsRng {
    k256::elliptic_curve::rand_core::OsRng
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
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
            self.store.lock().unwrap().get(cid).cloned().ok_or_else(|| {
                logos_messaging_a2a_storage::StorageError::Api {
                    status: 404,
                    body: format!("CID not found: {}", cid),
                }
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

    use logos_messaging_a2a_execution::TransferDetails;

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

    // ---- New coverage: from_key, with_config, respond, send_text, presence ----

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
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use logos_messaging_a2a_core::registry::InMemoryRegistry;
    use logos_messaging_a2a_transport::memory::InMemoryTransport;

    fn make_node(name: &str) -> WakuA2ANode<InMemoryTransport> {
        let transport = InMemoryTransport::new();
        WakuA2ANode::new(
            name,
            &format!("{} agent", name),
            vec!["test".into()],
            transport,
        )
    }

    #[tokio::test]
    async fn with_registry_builder() {
        let transport = InMemoryTransport::new();
        let registry = Arc::new(InMemoryRegistry::new());
        let node = WakuA2ANode::new("test", "test agent", vec![], transport)
            .with_registry(registry.clone());
        assert!(node.registry.is_some());
    }

    #[tokio::test]
    async fn register_in_registry_succeeds() {
        let node = make_node("echo");
        let registry = Arc::new(InMemoryRegistry::new());
        let node = node.with_registry(registry.clone());

        node.register_in_registry().await.unwrap();
        let card = registry.get(&node.card.public_key).await.unwrap();
        assert_eq!(card.name, "echo");
    }

    #[tokio::test]
    async fn register_without_registry_fails() {
        let node = make_node("echo");
        let result = node.register_in_registry().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no registry"));
    }

    #[tokio::test]
    async fn update_registry_succeeds() {
        let registry = Arc::new(InMemoryRegistry::new());
        let node = make_node("v1");
        let node = node.with_registry(registry.clone());
        node.register_in_registry().await.unwrap();

        // Simulate card update (change name field by re-registering after update)
        let mut updated_card = node.card.clone();
        updated_card.name = "v2".into();
        registry.update(updated_card.clone()).await.unwrap();

        let got = registry.get(&node.card.public_key).await.unwrap();
        assert_eq!(got.name, "v2");
    }

    #[tokio::test]
    async fn deregister_from_registry_succeeds() {
        let registry = Arc::new(InMemoryRegistry::new());
        let node = make_node("temp").with_registry(registry.clone());
        node.register_in_registry().await.unwrap();
        node.deregister_from_registry().await.unwrap();

        let result = registry.get(&node.card.public_key).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn discover_all_merges_waku_and_registry() {
        let transport = InMemoryTransport::new();
        let registry = Arc::new(InMemoryRegistry::new());

        // Register an agent in the registry
        let reg_card = AgentCard {
            name: "registry-agent".into(),
            description: "from registry".into(),
            version: "1.0.0".into(),
            capabilities: vec!["search".into()],
            public_key: "registry_key_001".into(),
            intro_bundle: None,
        };
        registry.register(reg_card).await.unwrap();

        let node =
            WakuA2ANode::new("discoverer", "disc", vec![], transport).with_registry(registry);

        let all = node.discover_all().await.unwrap();
        // Should find the registry agent
        assert!(all.iter().any(|c| c.name == "registry-agent"));
    }

    #[tokio::test]
    async fn discover_all_excludes_self_from_registry() {
        let transport = InMemoryTransport::new();
        let registry = Arc::new(InMemoryRegistry::new());

        let node =
            WakuA2ANode::new("self-node", "me", vec![], transport).with_registry(registry.clone());

        // Register self in registry
        node.register_in_registry().await.unwrap();

        let all = node.discover_all().await.unwrap();
        // Should NOT find self
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn discover_all_without_registry_returns_waku_only() {
        let node = make_node("plain");
        // No registry set — should still work, just return Waku results
        let all = node.discover_all().await.unwrap();
        assert!(all.is_empty()); // no Waku broadcasts either
    }
}
