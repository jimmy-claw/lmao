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

    // --- Additional edge-case coverage ---

    #[test]
    fn encrypt_empty_payload() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let encrypted = key.encrypt(b"").unwrap();
        let decrypted = key.decrypt(&encrypted).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn encrypt_large_payload() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let data = vec![0xab; 1024 * 1024]; // 1 MB
        let encrypted = key.encrypt(&data).unwrap();
        let decrypted = key.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn encrypt_single_byte() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let encrypted = key.encrypt(&[0x42]).unwrap();
        let decrypted = key.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, vec![0x42]);
    }

    #[test]
    fn decrypt_with_invalid_base64_nonce_fails() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let payload = EncryptedPayload {
            nonce: "not-valid-base64!!!".to_string(),
            ciphertext: "AAAA".to_string(),
        };
        let result = key.decrypt(&payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("base64"));
    }

    #[test]
    fn decrypt_with_invalid_base64_ciphertext_fails() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        // Valid base64 nonce (12 bytes = 16 base64 chars)
        let nonce_bytes = [0u8; 12];
        let nonce_b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes);
        let payload = EncryptedPayload {
            nonce: nonce_b64,
            ciphertext: "not-valid-base64!!!".to_string(),
        };
        let result = key.decrypt(&payload);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_with_tampered_ciphertext_fails() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let mut encrypted = key.encrypt(b"genuine message").unwrap();
        // Tamper with ciphertext by flipping bytes
        let mut ct_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &encrypted.ciphertext,
        )
        .unwrap();
        ct_bytes[0] ^= 0xff;
        encrypted.ciphertext =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, ct_bytes);

        let result = key.decrypt(&encrypted);
        assert!(result.is_err(), "tampered ciphertext must fail AEAD auth");
    }

    #[test]
    fn decrypt_with_tampered_nonce_fails() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let mut encrypted = key.encrypt(b"genuine message").unwrap();
        // Tamper with the nonce
        let mut nonce_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encrypted.nonce)
                .unwrap();
        nonce_bytes[0] ^= 0xff;
        encrypted.nonce =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes);

        let result = key.decrypt(&encrypted);
        assert!(result.is_err(), "tampered nonce must fail AEAD auth");
    }

    #[test]
    fn from_hex_invalid_hex_string_fails() {
        let result = AgentIdentity::from_hex("zzzz_not_hex");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("hex"));
    }

    #[test]
    fn from_hex_wrong_length_fails() {
        // 16 bytes (32 hex chars) instead of 32 bytes (64 hex chars)
        let result = AgentIdentity::from_hex("aabbccddaabbccddaabbccddaabbccdd");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("32 bytes"));
    }

    #[test]
    fn from_hex_empty_string_fails() {
        assert!(AgentIdentity::from_hex("").is_err());
    }

    #[test]
    fn parse_public_key_invalid_hex_fails() {
        let result = AgentIdentity::parse_public_key("not_hex!!!");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hex"));
    }

    #[test]
    fn parse_public_key_wrong_length_fails() {
        // 16 bytes instead of 32
        let result = AgentIdentity::parse_public_key("aabbccddaabbccddaabbccddaabbccdd");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("32 bytes"));
    }

    #[test]
    fn parse_public_key_empty_fails() {
        let result = AgentIdentity::parse_public_key("");
        assert!(result.is_err());
    }

    #[test]
    fn roundtrip_encrypt_decrypt_with_different_key_pairs() {
        // Verify that multiple independent key pairs produce independent sessions
        let a1 = AgentIdentity::generate();
        let b1 = AgentIdentity::generate();
        let a2 = AgentIdentity::generate();
        let b2 = AgentIdentity::generate();

        let key1 = a1.shared_key(&b1.public);
        let key2 = a2.shared_key(&b2.public);

        let enc1 = key1.encrypt(b"message for pair 1").unwrap();
        let enc2 = key2.encrypt(b"message for pair 2").unwrap();

        // Each key pair can only decrypt its own messages
        assert_eq!(key1.decrypt(&enc1).unwrap(), b"message for pair 1");
        assert_eq!(key2.decrypt(&enc2).unwrap(), b"message for pair 2");
        assert!(key1.decrypt(&enc2).is_err());
        assert!(key2.decrypt(&enc1).is_err());
    }

    #[test]
    fn cross_decrypt_ecdh_both_directions() {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();

        let key_ab = alice.shared_key(&bob.public);
        let key_ba = bob.shared_key(&alice.public);

        // Alice encrypts, Bob decrypts
        let enc = key_ab.encrypt(b"alice to bob").unwrap();
        assert_eq!(key_ba.decrypt(&enc).unwrap(), b"alice to bob");

        // Bob encrypts, Alice decrypts
        let enc2 = key_ba.encrypt(b"bob to alice").unwrap();
        assert_eq!(key_ab.decrypt(&enc2).unwrap(), b"bob to alice");
    }

    #[test]
    fn from_hex_produces_same_public_key() {
        // Use a known 32-byte secret
        let secret_hex = "a".repeat(64); // 32 bytes of 0xaa
        let identity1 = AgentIdentity::from_hex(&secret_hex).unwrap();
        let identity2 = AgentIdentity::from_hex(&secret_hex).unwrap();
        assert_eq!(identity1.public_key_hex(), identity2.public_key_hex());
    }

    #[test]
    fn encrypted_payload_json_roundtrip() {
        let payload = EncryptedPayload {
            nonce: "AAAAAAAAAAAAAAAA".to_string(),
            ciphertext: "Y2lwaGVydGV4dA==".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: EncryptedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, deserialized);
    }

    #[test]
    fn intro_bundle_version_is_1_0() {
        let bundle = IntroBundle::new("any_pubkey_hex");
        assert_eq!(bundle.version, "1.0");
        assert_eq!(bundle.agent_pubkey, "any_pubkey_hex");
    }

    #[test]
    fn intro_bundle_equality() {
        let a = IntroBundle::new("aabb");
        let b = IntroBundle::new("aabb");
        let c = IntroBundle::new("ccdd");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn public_key_hex_is_64_chars() {
        let identity = AgentIdentity::generate();
        let hex = identity.public_key_hex();
        assert_eq!(
            hex.len(),
            64,
            "X25519 public key is 32 bytes = 64 hex chars"
        );
        // All chars should be valid hex
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn different_identities_produce_different_public_keys() {
        let a = AgentIdentity::generate();
        let b = AgentIdentity::generate();
        assert_ne!(
            a.public_key_hex(),
            b.public_key_hex(),
            "random identities should have different public keys"
        );
    }
}
