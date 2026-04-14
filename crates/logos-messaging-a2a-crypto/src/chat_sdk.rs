//! Chat SDK session abstraction layer.
//!
//! Defines the [`SessionProvider`] trait — the interface that Logos Chat SDK's
//! Double Ratchet implementation will satisfy once Rust FFI bindings are
//! available. The [`EphemeralProvider`] provides the current X25519 + ChaCha20
//! crypto as a drop-in default so agents can encrypt today and upgrade to
//! forward-secrecy sessions without changing call sites.
//!
//! # Agent-to-agent use case (Issue #8)
//!
//! ```text
//! Agent A                          Agent B
//!   |-- IntroBundle (AgentCard) -->|
//!   |                              |
//!   |<-- open_session(pubkey) ---->|
//!   |                              |
//!   |== encrypt / decrypt ========>|  (per-task thread)
//!   |                              |
//!   |-- close_session ------------>|
//! ```
//!
//! When Chat SDK Rust FFI lands, implement [`SessionProvider`] with the
//! Double Ratchet backend and swap the provider at node init.

use crate::{AgentIdentity, CryptoError, EncryptedPayload, Result, SessionKey};

/// Unique identifier for an encrypted session between two agents.
pub type SessionId = String;

/// Metadata about an active session.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Unique session identifier.
    pub id: SessionId,
    /// Hex-encoded public key of the remote peer.
    pub peer_pubkey: String,
    /// Protocol version (e.g. `"ephemeral-x25519"` or `"chat-sdk-dr"` for
    /// Double Ratchet).
    pub protocol: String,
}

/// Trait abstracting encrypted session management.
///
/// Implementors provide key agreement, encrypt, and decrypt operations.
/// The current [`EphemeralProvider`] uses a single ECDH-derived key per
/// session. A future Chat SDK provider would use Double Ratchet state,
/// ratcheting keys on each message for forward secrecy.
pub trait SessionProvider: Send + Sync {
    /// Open (or resume) a session with the given peer public key.
    /// Returns a session ID that subsequent encrypt/decrypt calls reference.
    fn open_session(&self, peer_pubkey_hex: &str) -> Result<SessionId>;

    /// Encrypt plaintext within the given session.
    fn encrypt(&self, session_id: &str, plaintext: &[u8]) -> Result<EncryptedPayload>;

    /// Decrypt a payload within the given session.
    fn decrypt(&self, session_id: &str, payload: &EncryptedPayload) -> Result<Vec<u8>>;

    /// Close a session and clear any associated key material.
    fn close_session(&self, session_id: &str);

    /// List active sessions.
    fn sessions(&self) -> Vec<SessionInfo>;

    /// The protocol identifier for this provider (e.g. `"ephemeral-x25519"`).
    fn protocol(&self) -> &str;
}

/// Ephemeral session provider using X25519 ECDH + ChaCha20-Poly1305.
///
/// Each session derives a static shared key from a single ECDH exchange.
/// No ratcheting or forward secrecy — this is the stepping-stone implementation
/// that will be replaced by Chat SDK's Double Ratchet.
pub struct EphemeralProvider {
    identity: AgentIdentity,
    sessions: std::sync::Mutex<Vec<(SessionId, String, SessionKey)>>,
}

impl EphemeralProvider {
    /// Create a new provider with a fresh random identity.
    pub fn new() -> Self {
        Self::from_identity(AgentIdentity::generate())
    }

    /// Create a provider from an existing identity.
    pub fn from_identity(identity: AgentIdentity) -> Self {
        Self {
            identity,
            sessions: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The local agent's hex-encoded public key.
    pub fn public_key_hex(&self) -> String {
        self.identity.public_key_hex()
    }

}

impl Default for EphemeralProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionProvider for EphemeralProvider {
    fn open_session(&self, peer_pubkey_hex: &str) -> Result<SessionId> {
        let peer_pub = AgentIdentity::parse_public_key(peer_pubkey_hex)?;
        let key = self.identity.shared_key(&peer_pub);
        let id = format!(
            "eph-{}-{}",
            &self.identity.public_key_hex()[..8],
            &peer_pubkey_hex[..std::cmp::min(8, peer_pubkey_hex.len())]
        );
        let mut sessions = self.sessions.lock().unwrap();
        // Reuse existing session if already open for this peer
        if let Some(existing) = sessions.iter().find(|(_, pk, _)| pk == peer_pubkey_hex) {
            return Ok(existing.0.clone());
        }
        sessions.push((id.clone(), peer_pubkey_hex.to_string(), key));
        Ok(id)
    }

    fn encrypt(&self, session_id: &str, plaintext: &[u8]) -> Result<EncryptedPayload> {
        let sessions = self.sessions.lock().unwrap();
        let idx = sessions
            .iter()
            .position(|(id, _, _)| id == session_id)
            .ok_or_else(|| CryptoError::Cipher(format!("no such session: {session_id}")))?;
        sessions[idx].2.encrypt(plaintext)
    }

    fn decrypt(&self, session_id: &str, payload: &EncryptedPayload) -> Result<Vec<u8>> {
        let sessions = self.sessions.lock().unwrap();
        let idx = sessions
            .iter()
            .position(|(id, _, _)| id == session_id)
            .ok_or_else(|| CryptoError::Cipher(format!("no such session: {session_id}")))?;
        sessions[idx].2.decrypt(payload)
    }

    fn close_session(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|(id, _, _)| id != session_id);
    }

    fn sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .iter()
            .map(|(id, pk, _)| SessionInfo {
                id: id.clone(),
                peer_pubkey: pk.clone(),
                protocol: self.protocol().to_string(),
            })
            .collect()
    }

    fn protocol(&self) -> &str {
        "ephemeral-x25519"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_roundtrip() {
        let alice = EphemeralProvider::new();
        let bob = EphemeralProvider::new();

        let sid_a = alice.open_session(&bob.public_key_hex()).unwrap();
        let sid_b = bob.open_session(&alice.public_key_hex()).unwrap();

        let enc = alice.encrypt(&sid_a, b"hello from alice").unwrap();
        let dec = bob.decrypt(&sid_b, &enc).unwrap();
        assert_eq!(dec, b"hello from alice");
    }

    #[test]
    fn bidirectional_communication() {
        let alice = EphemeralProvider::new();
        let bob = EphemeralProvider::new();

        let sid_a = alice.open_session(&bob.public_key_hex()).unwrap();
        let sid_b = bob.open_session(&alice.public_key_hex()).unwrap();

        // Alice -> Bob
        let enc = alice.encrypt(&sid_a, b"ping").unwrap();
        assert_eq!(bob.decrypt(&sid_b, &enc).unwrap(), b"ping");

        // Bob -> Alice
        let enc = bob.encrypt(&sid_b, b"pong").unwrap();
        assert_eq!(alice.decrypt(&sid_a, &enc).unwrap(), b"pong");
    }

    #[test]
    fn close_session_removes_it() {
        let alice = EphemeralProvider::new();
        let bob = EphemeralProvider::new();

        let sid = alice.open_session(&bob.public_key_hex()).unwrap();
        assert_eq!(alice.sessions().len(), 1);

        alice.close_session(&sid);
        assert!(alice.sessions().is_empty());
        assert!(alice.encrypt(&sid, b"should fail").is_err());
    }

    #[test]
    fn open_duplicate_returns_same_id() {
        let alice = EphemeralProvider::new();
        let bob = EphemeralProvider::new();

        let sid1 = alice.open_session(&bob.public_key_hex()).unwrap();
        let sid2 = alice.open_session(&bob.public_key_hex()).unwrap();
        assert_eq!(sid1, sid2);
        assert_eq!(alice.sessions().len(), 1);
    }

    #[test]
    fn invalid_session_id_fails() {
        let alice = EphemeralProvider::new();
        assert!(alice.encrypt("nonexistent", b"data").is_err());
        assert!(alice
            .decrypt(
                "nonexistent",
                &EncryptedPayload {
                    nonce: String::new(),
                    ciphertext: String::new(),
                }
            )
            .is_err());
    }

    #[test]
    fn invalid_peer_pubkey_fails() {
        let alice = EphemeralProvider::new();
        assert!(alice.open_session("not-hex").is_err());
        assert!(alice.open_session("aabb").is_err()); // too short
    }

    #[test]
    fn session_info_fields() {
        let alice = EphemeralProvider::new();
        let bob = EphemeralProvider::new();

        alice.open_session(&bob.public_key_hex()).unwrap();
        let info = &alice.sessions()[0];
        assert_eq!(info.peer_pubkey, bob.public_key_hex());
        assert_eq!(info.protocol, "ephemeral-x25519");
        assert!(info.id.starts_with("eph-"));
    }

    #[test]
    fn protocol_identifier() {
        let p = EphemeralProvider::new();
        assert_eq!(p.protocol(), "ephemeral-x25519");
    }

    #[test]
    fn multiple_sessions() {
        let alice = EphemeralProvider::new();
        let bob = EphemeralProvider::new();
        let carol = EphemeralProvider::new();

        alice.open_session(&bob.public_key_hex()).unwrap();
        alice.open_session(&carol.public_key_hex()).unwrap();
        assert_eq!(alice.sessions().len(), 2);
    }

    #[test]
    fn default_creates_provider() {
        let p = EphemeralProvider::default();
        assert_eq!(p.protocol(), "ephemeral-x25519");
        assert!(p.sessions().is_empty());
    }
}
