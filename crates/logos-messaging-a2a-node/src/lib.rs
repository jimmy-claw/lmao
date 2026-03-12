//! Main A2A node implementation for the Logos Messaging protocol.
//!
//! This crate combines crypto ([`logos_messaging_a2a_crypto`]), transport
//! ([`logos_messaging_a2a_transport`]), storage ([`logos_messaging_a2a_storage`]),
//! and execution ([`logos_messaging_a2a_execution`]) into a single high-level
//! [`WakuA2ANode`] that can announce, discover, send/receive tasks, manage
//! sessions, stream responses, and handle payments over a Waku-compatible
//! transport layer.

pub mod discovery;
pub mod payment;
pub mod presence;
pub mod retry;
pub mod session;
pub mod storage;
mod streaming;
mod tasks;

use anyhow::{Context, Result};
use k256::ecdsa::SigningKey;
use logos_messaging_a2a_core::registry::AgentRegistry;
use logos_messaging_a2a_core::AgentCard;
use logos_messaging_a2a_core::RetryConfig;
/// Re-export of [`logos_messaging_a2a_core::Task`] for convenience.
pub use logos_messaging_a2a_core::Task as TaskType;
use logos_messaging_a2a_crypto::{AgentIdentity, IntroBundle};
use logos_messaging_a2a_transport::sds::{ChannelConfig, MessageChannel};
use logos_messaging_a2a_transport::Transport;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Re-export of [`session::Session`] for convenient access from crate root.
pub use session::Session;

/// Re-export of [`storage::StorageOffloadConfig`] for convenient access from crate root.
pub use storage::StorageOffloadConfig;

/// Re-export of [`payment::PaymentConfig`] for convenient access from crate root.
pub use payment::PaymentConfig;

/// A2A node: announce, discover, send/receive tasks over Waku.
///
/// Uses SDS MessageChannel for reliable, causally-ordered delivery with
/// bloom filter deduplication and implicit ACK via remote bloom filters.
pub struct WakuA2ANode<T: Transport> {
    /// This agent's public identity card, including name, capabilities, and public key.
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
    /// Buffered stream chunks keyed by task_id, sorted by chunk_index.
    stream_chunks:
        std::sync::Mutex<HashMap<String, Vec<logos_messaging_a2a_core::TaskStreamChunk>>>,
    /// Optional retry configuration for exponential-backoff retries of
    /// transport send failures.
    retry_config: Option<RetryConfig>,
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
            stream_chunks: std::sync::Mutex::new(HashMap::new()),
            retry_config: None,
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
            stream_chunks: std::sync::Mutex::new(HashMap::new()),
            retry_config: None,
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
            stream_chunks: std::sync::Mutex::new(HashMap::new()),
            retry_config: None,
        }
    }

    /// Alias for [`from_key`](Self::from_key) — create a node from an existing signing key.
    pub fn new_with_key(
        name: &str,
        description: &str,
        capabilities: Vec<String>,
        transport: T,
        signing_key: SigningKey,
    ) -> Self {
        Self::from_key(name, description, capabilities, transport, signing_key)
    }

    /// Load a signing key from a file (hex-encoded 32 bytes), or generate and
    /// save one if the file does not exist. File is created with mode 0600.
    pub fn from_keyfile(
        name: &str,
        description: &str,
        capabilities: Vec<String>,
        transport: T,
        path: &Path,
    ) -> Result<Self> {
        let signing_key = if path.exists() {
            let hex_str = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read keyfile {}", path.display()))?;
            let bytes = hex::decode(hex_str.trim())
                .with_context(|| format!("invalid hex in keyfile {}", path.display()))?;
            if bytes.len() != 32 {
                anyhow::bail!(
                    "keyfile {} contains {} bytes, expected 32",
                    path.display(),
                    bytes.len()
                );
            }
            SigningKey::from_bytes(bytes.as_slice().into())
                .with_context(|| format!("invalid signing key in {}", path.display()))?
        } else {
            let key = SigningKey::random(&mut rand_core());
            let hex_str = hex::encode(key.to_bytes());

            // Write atomically: create file, set perms, write content
            {
                use std::io::Write;
                let mut file = std::fs::File::create(path)
                    .with_context(|| format!("failed to create keyfile {}", path.display()))?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(std::fs::Permissions::from_mode(0o600))
                        .with_context(|| {
                            format!("failed to set permissions on {}", path.display())
                        })?;
                }

                file.write_all(hex_str.as_bytes())
                    .with_context(|| format!("failed to write keyfile {}", path.display()))?;
            }

            key
        };

        Ok(Self::from_key(
            name,
            description,
            capabilities,
            transport,
            signing_key,
        ))
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
            stream_chunks: std::sync::Mutex::new(HashMap::new()),
            retry_config: None,
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

    /// Enable x402-style payment flow via an [`ExecutionBackend`](logos_messaging_a2a_execution::ExecutionBackend).
    ///
    /// When configured, outgoing tasks can auto-pay and incoming tasks can
    /// require payment proof before processing.
    pub fn with_payment(mut self, config: PaymentConfig) -> Self {
        self.payment = Some(config);
        self
    }

    /// Enable exponential-backoff retry for transport send failures.
    ///
    /// When configured, `send_task` / `send_task_to` will retry on transport
    /// errors up to `config.max_attempts` times with exponential backoff.
    pub fn with_retry(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
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

    /// Get the current retry configuration, if any.
    pub fn retry_config(&self) -> Option<&RetryConfig> {
        self.retry_config.as_ref()
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
}

/// Platform-appropriate RNG.
fn rand_core() -> k256::elliptic_curve::rand_core::OsRng {
    k256::elliptic_curve::rand_core::OsRng
}

#[cfg(test)]
mod keyfile_tests {
    use super::*;
    use logos_messaging_a2a_transport::memory::InMemoryTransport;
    use tempfile::TempDir;

    fn make_transport() -> InMemoryTransport {
        InMemoryTransport::new()
    }

    #[test]
    fn new_with_key_creates_node_with_specified_key() {
        let key = SigningKey::random(&mut rand_core());
        let expected_pubkey = hex::encode(key.verifying_key().to_encoded_point(true).as_bytes());

        let node = WakuA2ANode::new_with_key(
            "test",
            "test agent",
            vec!["text".into()],
            make_transport(),
            key,
        );

        assert_eq!(node.pubkey(), expected_pubkey);
        assert_eq!(node.card.name, "test");
    }

    #[test]
    fn from_keyfile_creates_file_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.key");

        assert!(!path.exists());

        let node = WakuA2ANode::from_keyfile(
            "test",
            "test agent",
            vec!["text".into()],
            make_transport(),
            &path,
        )
        .unwrap();

        assert!(path.exists());
        // File should contain 64 hex chars (32 bytes)
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.len(), 64);
        hex::decode(&contents).expect("file should be valid hex");

        // Node should have a valid pubkey
        assert!(!node.pubkey().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn from_keyfile_sets_permissions_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.key");

        WakuA2ANode::from_keyfile("test", "test agent", vec![], make_transport(), &path).unwrap();

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[test]
    fn from_keyfile_loads_existing_key_same_pubkey() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.key");

        let node1 = WakuA2ANode::from_keyfile(
            "agent-a",
            "first",
            vec!["text".into()],
            make_transport(),
            &path,
        )
        .unwrap();
        let pubkey1 = node1.pubkey().to_string();

        let node2 = WakuA2ANode::from_keyfile(
            "agent-b",
            "second",
            vec!["code".into()],
            make_transport(),
            &path,
        )
        .unwrap();
        let pubkey2 = node2.pubkey().to_string();

        assert_eq!(pubkey1, pubkey2, "reloaded key should produce same pubkey");
    }

    #[test]
    fn from_keyfile_roundtrip_restart_same_identity() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.key");

        // "First run" — generates key
        let pubkey_first = {
            let node = WakuA2ANode::from_keyfile(
                "bot",
                "my bot",
                vec!["text".into()],
                make_transport(),
                &path,
            )
            .unwrap();
            node.pubkey().to_string()
        };
        // node is dropped — simulates process exit

        // "Second run" — loads existing key
        let pubkey_second = {
            let node = WakuA2ANode::from_keyfile(
                "bot",
                "my bot",
                vec!["text".into()],
                make_transport(),
                &path,
            )
            .unwrap();
            node.pubkey().to_string()
        };

        assert_eq!(pubkey_first, pubkey_second, "identity must survive restart");
    }

    #[test]
    fn from_keyfile_rejects_wrong_length() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.key");

        // Write 16 bytes (too short)
        std::fs::write(&path, "aabbccdd00112233aabbccdd00112233").unwrap();

        match WakuA2ANode::from_keyfile("test", "test", vec![], make_transport(), &path) {
            Err(e) => assert!(
                e.to_string().contains("16 bytes, expected 32"),
                "error should mention length: {}",
                e
            ),
            Ok(_) => panic!("expected error for wrong-length key"),
        }
    }

    #[test]
    fn from_keyfile_rejects_invalid_hex() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.key");

        std::fs::write(&path, "not-valid-hex!!").unwrap();

        match WakuA2ANode::from_keyfile("test", "test", vec![], make_transport(), &path) {
            Err(e) => assert!(
                e.to_string().contains("invalid hex"),
                "error should mention hex: {}",
                e
            ),
            Ok(_) => panic!("expected error for invalid hex"),
        }
    }

    #[test]
    fn from_keyfile_with_known_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("known.key");

        // Generate a key, write it manually, then load via from_keyfile
        let key = SigningKey::random(&mut rand_core());
        let hex_str = hex::encode(key.to_bytes());
        let expected_pubkey = hex::encode(key.verifying_key().to_encoded_point(true).as_bytes());
        std::fs::write(&path, &hex_str).unwrap();

        let node =
            WakuA2ANode::from_keyfile("test", "test", vec![], make_transport(), &path).unwrap();

        assert_eq!(node.pubkey(), expected_pubkey);
    }
}
