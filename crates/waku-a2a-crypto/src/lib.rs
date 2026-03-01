use anyhow::{Context, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};

/// Agent identity keypair (X25519 for ECDH key agreement).
pub struct AgentIdentity {
    secret: StaticSecret,
    pub public: PublicKey,
}

impl AgentIdentity {
    /// Generate a new random identity.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Hex-encoded public key.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public.as_bytes())
    }

    /// Reconstruct from hex-encoded secret key (32 bytes = 64 hex chars). For testing.
    pub fn from_hex(secret_hex: &str) -> Result<Self> {
        let bytes = hex::decode(secret_hex).context("invalid hex for secret key")?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("secret key must be 32 bytes"))?;
        let secret = StaticSecret::from(arr);
        let public = PublicKey::from(&secret);
        Ok(Self { secret, public })
    }

    /// Parse a hex-encoded X25519 public key.
    pub fn parse_public_key(hex_str: &str) -> Result<PublicKey> {
        let bytes = hex::decode(hex_str).context("invalid hex for public key")?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;
        Ok(PublicKey::from(arr))
    }

    /// ECDH key agreement → shared secret → ChaCha20-Poly1305 key.
    pub fn shared_key(&self, their_pubkey: &PublicKey) -> SessionKey {
        let shared = self.secret.diffie_hellman(their_pubkey);
        SessionKey(*shared.as_bytes())
    }
}

/// Symmetric session key derived from ECDH.
pub struct SessionKey([u8; 32]);

impl SessionKey {
    /// Encrypt plaintext, returns EncryptedPayload with random nonce.
    #[allow(deprecated)]
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload> {
        let cipher = ChaCha20Poly1305::new_from_slice(&self.0)
            .map_err(|e| anyhow::anyhow!("cipher init: {}", e))?;

        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("encrypt: {}", e))?;

        Ok(EncryptedPayload {
            nonce: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
            ciphertext: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                ciphertext,
            ),
        })
    }

    /// Decrypt an EncryptedPayload, returns plaintext bytes.
    #[allow(deprecated)]
    pub fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new_from_slice(&self.0)
            .map_err(|e| anyhow::anyhow!("cipher init: {}", e))?;

        let nonce_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &payload.nonce)
                .context("invalid base64 nonce")?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &payload.ciphertext,
        )
        .context("invalid base64 ciphertext")?;

        cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|e| anyhow::anyhow!("decrypt: {}", e))
    }
}

/// Encrypted payload with base64-encoded nonce and ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncryptedPayload {
    pub nonce: String,
    pub ciphertext: String,
}

/// Introduction bundle — shared out-of-band to establish an encrypted session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntroBundle {
    pub agent_pubkey: String,
    pub version: String,
}

impl IntroBundle {
    pub fn new(agent_pubkey: &str) -> Self {
        Self {
            agent_pubkey: agent_pubkey.to_string(),
            version: "1.0".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();

        let key = alice.shared_key(&bob.public);
        let plaintext = b"Hello, encrypted world!";
        let encrypted = key.encrypt(plaintext).unwrap();
        let decrypted = key.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn ecdh_shared_secret_symmetric() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();

        let key_ab = alice.shared_key(&bob.public);
        let key_ba = bob.shared_key(&alice.public);

        let plaintext = b"symmetric test";
        let encrypted = key_ab.encrypt(plaintext).unwrap();
        let decrypted = key_ba.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn intro_bundle_serialization() {
        let bundle = IntroBundle::new("aabbccdd");
        let json = serde_json::to_string(&bundle).unwrap();
        let deserialized: IntroBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(bundle, deserialized);
        assert!(json.contains("aabbccdd"));
        assert!(json.contains("1.0"));
    }

    #[test]
    fn different_nonce_each_encrypt() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let e1 = key.encrypt(b"same plaintext").unwrap();
        let e2 = key.encrypt(b"same plaintext").unwrap();
        assert_ne!(e1.nonce, e2.nonce, "nonce must be random each time");
    }

    #[test]
    fn from_hex_roundtrip() {
        let alice = AgentIdentity::generate();
        let hex_pub = alice.public_key_hex();
        let parsed = AgentIdentity::parse_public_key(&hex_pub).unwrap();
        assert_eq!(parsed.as_bytes(), alice.public.as_bytes());
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let eve = AgentIdentity::generate();

        let key_ab = alice.shared_key(&bob.public);
        let key_ae = alice.shared_key(&eve.public);

        let encrypted = key_ab.encrypt(b"secret").unwrap();
        assert!(key_ae.decrypt(&encrypted).is_err());
    }
}
