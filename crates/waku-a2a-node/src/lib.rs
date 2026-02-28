use anyhow::{Context, Result};
use k256::ecdsa::SigningKey;
use tokio::sync::mpsc;
use waku_a2a_core::{topics, A2AEnvelope, AgentCard, Task};
use waku_a2a_crypto::{AgentIdentity, IntroBundle};
use waku_a2a_transport::sds::SdsTransport;
use waku_a2a_transport::Transport;

/// A2A node: announce, discover, send/receive tasks over Waku.
pub struct WakuA2ANode<T: Transport> {
    pub card: AgentCard,
    transport: SdsTransport<T>,
    signing_key: SigningKey,
    /// Optional X25519 identity for encrypted sessions.
    identity: Option<AgentIdentity>,
    /// Persistent subscription to this node's task topic (lazy-initialized).
    task_rx: tokio::sync::Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
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
            public_key,
            intro_bundle: None,
        };

        Self {
            card,
            transport: SdsTransport::new(transport),
            signing_key,
            identity: None,
            task_rx: tokio::sync::Mutex::new(None),
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
            public_key,
            intro_bundle: Some(intro_bundle),
        };

        Self {
            card,
            transport: SdsTransport::new(transport),
            signing_key,
            identity: Some(identity),
            task_rx: tokio::sync::Mutex::new(None),
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
            public_key,
            intro_bundle: None,
        };

        Self {
            card,
            transport: SdsTransport::new(transport),
            signing_key,
            identity: None,
            task_rx: tokio::sync::Mutex::new(None),
        }
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

    /// Broadcast this agent's card on the discovery topic.
    pub async fn announce(&self) -> Result<()> {
        let envelope = A2AEnvelope::AgentCard(self.card.clone());
        let payload = serde_json::to_vec(&envelope).context("Failed to serialize AgentCard")?;
        self.transport
            .inner()
            .publish(topics::DISCOVERY, &payload)
            .await
            .context("Failed to announce AgentCard")?;
        eprintln!("[node] Announced: {} ({})", self.card.name, self.pubkey());
        Ok(())
    }

    /// Discover agents by subscribing to the discovery topic and draining messages.
    pub async fn discover(&self) -> Result<Vec<AgentCard>> {
        let mut rx = self.transport.inner().subscribe(topics::DISCOVERY).await?;

        let mut cards = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Ok(A2AEnvelope::AgentCard(card)) = serde_json::from_slice(&msg) {
                if card.public_key != self.card.public_key {
                    cards.push(card);
                }
            }
        }

        let _ = self.transport.inner().unsubscribe(topics::DISCOVERY).await;
        Ok(cards)
    }

    /// Send a task to another agent. Uses SDS for reliable delivery.
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

        let envelope = self.maybe_encrypt_task(task, recipient_card)?;
        let payload = serde_json::to_vec(&envelope).context("Failed to serialize envelope")?;

        let acked = self
            .transport
            .publish_reliable(&topic, &payload, &task.id)
            .await
            .context("SDS publish failed")?;

        if acked {
            eprintln!("[node] Task {} sent and ACKed", task.id);
        } else {
            eprintln!("[node] Task {} sent but no ACK received", task.id);
        }
        Ok(acked)
    }

    /// Poll for incoming tasks addressed to this agent.
    /// Lazily subscribes to the task topic on first call.
    /// Automatically decrypts encrypted tasks if this node has an identity.
    pub async fn poll_tasks(&self) -> Result<Vec<Task>> {
        let raw_messages = {
            let mut task_rx = self.task_rx.lock().await;
            if task_rx.is_none() {
                let topic = topics::task_topic(&self.card.public_key);
                *task_rx = Some(self.transport.inner().subscribe(&topic).await?);
            }
            let rx = task_rx.as_mut().unwrap();

            let mut msgs = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                msgs.push(msg);
            }
            msgs
        };

        let messages = self.transport.filter_dedup(raw_messages);
        let mut tasks = Vec::new();

        for msg in messages {
            if let Ok(envelope) = serde_json::from_slice::<A2AEnvelope>(&msg) {
                match envelope {
                    A2AEnvelope::Task(task) => {
                        let _ = self.transport.send_ack(&task.id).await;
                        tasks.push(task);
                    }
                    A2AEnvelope::EncryptedTask {
                        encrypted,
                        sender_pubkey,
                    } => {
                        if let Some(ref identity) = self.identity {
                            match self.decrypt_task(identity, &sender_pubkey, &encrypted) {
                                Ok(task) => {
                                    let _ = self.transport.send_ack(&task.id).await;
                                    tasks.push(task);
                                }
                                Err(e) => {
                                    eprintln!("[node] Failed to decrypt task: {}", e);
                                }
                            }
                        } else {
                            eprintln!("[node] Received encrypted task but no identity configured");
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(tasks)
    }

    /// Respond to a task: send back a completed task with result.
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

        let envelope = self.maybe_encrypt_task(&response, sender_card)?;
        let payload = serde_json::to_vec(&envelope)?;

        self.transport
            .inner()
            .publish(&topic, &payload)
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
        encrypted: &waku_a2a_crypto::EncryptedPayload,
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
}
