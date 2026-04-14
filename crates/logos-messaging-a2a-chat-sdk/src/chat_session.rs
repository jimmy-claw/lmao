//! Pairwise encrypted agent session backed by Chat SDK crypto primitives.

use logos_messaging_a2a_crypto::{
    AgentIdentity, CryptoError, EncryptedPayload, IntroBundle, SessionKey,
};
use serde::{Deserialize, Serialize};

/// Errors from [`ChatSession`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ChatSessionError {
    #[error("session not established — call establish() with peer intro bundle first")]
    NotEstablished,
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
}

/// A pairwise encrypted session between two agents.
///
/// Wraps the Chat SDK crypto layer (X25519 ECDH + ChaCha20-Poly1305) in a
/// session-oriented API suitable for agent-to-agent communication.
///
/// # Usage
///
/// ```rust
/// use logos_messaging_a2a_chat_sdk::{ChatSession, IntroBundle};
///
/// // Agent A creates a session and shares its intro bundle
/// let mut alice = ChatSession::new("alice-agent");
/// let alice_bundle = alice.intro_bundle();
///
/// // Agent B receives Alice's bundle and establishes a session
/// let mut bob = ChatSession::new("bob-agent");
/// let bob_bundle = bob.intro_bundle();
/// bob.establish(&alice_bundle).unwrap();
///
/// // Alice establishes her side too
/// alice.establish(&bob_bundle).unwrap();
///
/// // Now both can encrypt/decrypt messages
/// let encrypted = alice.encrypt(b"hello from alice").unwrap();
/// let decrypted = bob.decrypt(&encrypted).unwrap();
/// assert_eq!(decrypted, b"hello from alice");
/// ```
pub struct ChatSession {
    /// Human-readable agent name (for logging / debugging).
    name: String,
    /// This agent's X25519 identity.
    identity: AgentIdentity,
    /// Derived session key (populated after establish()).
    session_key: Option<SessionKey>,
    /// Peer's public key hex (populated after establish()).
    peer_pubkey: Option<String>,
    /// Unique session identifier.
    session_id: String,
}

/// Serializable session metadata (for persistence or wire transfer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSessionInfo {
    pub session_id: String,
    pub agent_name: String,
    pub agent_pubkey: String,
    pub peer_pubkey: Option<String>,
    pub established: bool,
}

impl ChatSession {
    /// Create a new session with a fresh X25519 identity.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            identity: AgentIdentity::generate(),
            session_key: None,
            peer_pubkey: None,
            session_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Create a session from an existing identity (e.g. loaded from keyfile).
    pub fn from_identity(name: &str, identity: AgentIdentity) -> Self {
        Self {
            name: name.to_string(),
            identity,
            session_key: None,
            peer_pubkey: None,
            session_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// This agent's intro bundle — share this with peers for session establishment.
    pub fn intro_bundle(&self) -> IntroBundle {
        IntroBundle::new(&self.identity.public_key_hex())
    }

    /// Establish the session using a peer's intro bundle.
    ///
    /// Performs X25519 ECDH key agreement to derive a shared ChaCha20-Poly1305
    /// session key. Both sides must call this with the other's intro bundle.
    pub fn establish(&mut self, peer_bundle: &IntroBundle) -> Result<(), ChatSessionError> {
        let peer_pub = AgentIdentity::parse_public_key(&peer_bundle.agent_pubkey)?;
        let session_key = self.identity.shared_key(&peer_pub);
        self.session_key = Some(session_key);
        self.peer_pubkey = Some(peer_bundle.agent_pubkey.clone());
        Ok(())
    }

    /// Whether the session has been established with a peer.
    pub fn is_established(&self) -> bool {
        self.session_key.is_some()
    }

    /// Encrypt a plaintext message for the peer.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload, ChatSessionError> {
        let key = self.session_key.as_ref().ok_or(ChatSessionError::NotEstablished)?;
        Ok(key.encrypt(plaintext)?)
    }

    /// Decrypt a message from the peer.
    pub fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>, ChatSessionError> {
        let key = self.session_key.as_ref().ok_or(ChatSessionError::NotEstablished)?;
        Ok(key.decrypt(payload)?)
    }

    /// Session metadata (serializable).
    pub fn info(&self) -> ChatSessionInfo {
        ChatSessionInfo {
            session_id: self.session_id.clone(),
            agent_name: self.name.clone(),
            agent_pubkey: self.identity.public_key_hex(),
            peer_pubkey: self.peer_pubkey.clone(),
            established: self.is_established(),
        }
    }

    /// The unique session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// This agent's X25519 public key as hex.
    pub fn pubkey_hex(&self) -> String {
        self.identity.public_key_hex()
    }

    /// Peer's X25519 public key hex, if the session is established.
    pub fn peer_pubkey(&self) -> Option<&str> {
        self.peer_pubkey.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_not_established() {
        let session = ChatSession::new("test");
        assert!(!session.is_established());
        assert!(session.peer_pubkey().is_none());
    }

    #[test]
    fn establish_bilateral_session() {
        let mut alice = ChatSession::new("alice");
        let mut bob = ChatSession::new("bob");

        let alice_bundle = alice.intro_bundle();
        let bob_bundle = bob.intro_bundle();

        alice.establish(&bob_bundle).unwrap();
        bob.establish(&alice_bundle).unwrap();

        assert!(alice.is_established());
        assert!(bob.is_established());
        assert_eq!(alice.peer_pubkey().unwrap(), bob.pubkey_hex());
        assert_eq!(bob.peer_pubkey().unwrap(), alice.pubkey_hex());
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let mut alice = ChatSession::new("alice");
        let mut bob = ChatSession::new("bob");

        alice.establish(&bob.intro_bundle()).unwrap();
        bob.establish(&alice.intro_bundle()).unwrap();

        let msg = b"hello from alice";
        let encrypted = alice.encrypt(msg).unwrap();
        let decrypted = bob.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, msg);
    }

    #[test]
    fn encrypt_before_establish_fails() {
        let session = ChatSession::new("lonely");
        let result = session.encrypt(b"hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not established"));
    }

    #[test]
    fn decrypt_before_establish_fails() {
        let session = ChatSession::new("lonely");
        let payload = EncryptedPayload {
            nonce: "AAAAAAAAAAAAAAAA".to_string(),
            ciphertext: "AAAA".to_string(),
        };
        let result = session.decrypt(&payload);
        assert!(result.is_err());
    }

    #[test]
    fn bidirectional_messaging() {
        let mut alice = ChatSession::new("alice");
        let mut bob = ChatSession::new("bob");

        alice.establish(&bob.intro_bundle()).unwrap();
        bob.establish(&alice.intro_bundle()).unwrap();

        // Alice -> Bob
        let enc = alice.encrypt(b"alice msg").unwrap();
        assert_eq!(bob.decrypt(&enc).unwrap(), b"alice msg");

        // Bob -> Alice
        let enc = bob.encrypt(b"bob msg").unwrap();
        assert_eq!(alice.decrypt(&enc).unwrap(), b"bob msg");
    }

    #[test]
    fn session_info_serialization() {
        let mut session = ChatSession::new("test-agent");
        let info = session.info();
        assert_eq!(info.agent_name, "test-agent");
        assert!(!info.established);
        assert!(info.peer_pubkey.is_none());

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: ChatSessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_id, info.session_id);

        // After establish
        let peer = ChatSession::new("peer");
        session.establish(&peer.intro_bundle()).unwrap();
        let info = session.info();
        assert!(info.established);
        assert!(info.peer_pubkey.is_some());
    }

    #[test]
    fn from_identity_preserves_pubkey() {
        let identity = AgentIdentity::generate();
        let expected_hex = identity.public_key_hex();
        let session = ChatSession::from_identity("loaded", identity);
        assert_eq!(session.pubkey_hex(), expected_hex);
    }

    #[test]
    fn session_id_is_uuid() {
        let session = ChatSession::new("test");
        let id = session.session_id();
        // UUID v4 format: 8-4-4-4-12
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
    }

    #[test]
    fn establish_with_invalid_pubkey_fails() {
        let mut session = ChatSession::new("test");
        let bad_bundle = IntroBundle::new("not_valid_hex");
        let result = session.establish(&bad_bundle);
        assert!(result.is_err());
        assert!(!session.is_established());
    }

    #[test]
    fn wrong_peer_cannot_decrypt() {
        let mut alice = ChatSession::new("alice");
        let mut bob = ChatSession::new("bob");
        let mut eve = ChatSession::new("eve");

        alice.establish(&bob.intro_bundle()).unwrap();
        bob.establish(&alice.intro_bundle()).unwrap();
        eve.establish(&alice.intro_bundle()).unwrap();

        let encrypted = alice.encrypt(b"secret").unwrap();
        assert_eq!(bob.decrypt(&encrypted).unwrap(), b"secret");
        assert!(eve.decrypt(&encrypted).is_err());
    }

    #[test]
    fn multiple_messages_in_session() {
        let mut alice = ChatSession::new("alice");
        let mut bob = ChatSession::new("bob");

        alice.establish(&bob.intro_bundle()).unwrap();
        bob.establish(&alice.intro_bundle()).unwrap();

        for i in 0..10 {
            let msg = format!("message {}", i);
            let enc = alice.encrypt(msg.as_bytes()).unwrap();
            let dec = bob.decrypt(&enc).unwrap();
            assert_eq!(dec, msg.as_bytes());
        }
    }
}
