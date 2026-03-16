use k256::ecdsa::signature::Verifier;
use k256::ecdsa::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Errors that can occur during presence verification.
#[derive(Debug, thiserror::Error)]
pub enum PresenceError {
    #[error("missing signature")]
    MissingSignature,

    #[error("agent_id is not valid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),

    #[error("agent_id is not a valid secp256k1 public key: {0}")]
    InvalidPublicKey(#[source] k256::ecdsa::Error),

    #[error("signature is not valid DER: {0}")]
    InvalidSignature(#[source] k256::ecdsa::Error),

    #[error("signature verification failed: {0}")]
    VerificationFailed(#[source] k256::ecdsa::Error),
}

/// Presence announcement broadcast on the well-known presence topic.
///
/// Agents periodically publish these so peers can build a live map of
/// available agents and their capabilities without querying a registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresenceAnnouncement {
    /// Agent public key (secp256k1 compressed hex) — unique identity.
    pub agent_id: String,
    /// Human-readable agent name.
    pub name: String,
    /// Capabilities this agent offers (e.g. `["summarize", "translate"]`).
    pub capabilities: Vec<String>,
    /// Waku content topic where this agent receives tasks.
    pub waku_topic: String,
    /// How long (seconds) this announcement is valid. Peers should evict
    /// entries older than `ttl_secs` without a refresh.
    pub ttl_secs: u64,
    /// Optional secp256k1 signature over the canonical JSON of the other
    /// fields, proving the announcement comes from the claimed `agent_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

impl PresenceAnnouncement {
    /// Deterministic serialization of all fields except `signature`.
    ///
    /// Produces a canonical JSON object with keys in a fixed order so that
    /// both signer and verifier hash identical bytes.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // Build canonical JSON manually with deterministic key order.
        let canonical = serde_json::json!({
            "agent_id": self.agent_id,
            "capabilities": self.capabilities,
            "name": self.name,
            "ttl_secs": self.ttl_secs,
            "waku_topic": self.waku_topic,
        });
        serde_json::to_vec(&canonical).expect("canonical JSON serialization cannot fail")
    }

    /// Sign this announcement with a secp256k1 signing key.
    ///
    /// Computes `canonical_bytes()` and signs with the provided key,
    /// setting the `signature` field to the DER-encoded signature bytes.
    pub fn sign(&mut self, signing_key: &SigningKey) -> Result<(), PresenceError> {
        use k256::ecdsa::signature::Signer;
        let message = self.canonical_bytes();
        let sig: Signature = signing_key.sign(&message);
        self.signature = Some(sig.to_der().as_bytes().to_vec());
        Ok(())
    }

    /// Verify the signature against `agent_id` (compressed secp256k1 pubkey hex).
    ///
    /// Returns `Ok(())` if the signature is present and valid, or an error
    /// describing why verification failed.
    pub fn verify(&self) -> Result<(), PresenceError> {
        let sig_bytes = self
            .signature
            .as_ref()
            .ok_or(PresenceError::MissingSignature)?;

        let pubkey_bytes = hex::decode(&self.agent_id)?;
        let verifying_key = VerifyingKey::from_sec1_bytes(&pubkey_bytes)
            .map_err(PresenceError::InvalidPublicKey)?;

        let signature = Signature::from_der(sig_bytes).map_err(PresenceError::InvalidSignature)?;

        let message = self.canonical_bytes();
        verifying_key
            .verify(&message, &signature)
            .map_err(PresenceError::VerificationFailed)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presence_announcement_serialization() {
        let ann = PresenceAnnouncement {
            agent_id: "02abcdef".to_string(),
            name: "echo".to_string(),
            capabilities: vec!["text".to_string(), "summarize".to_string()],
            waku_topic: "/waku-a2a/1/task/02abcdef/proto".to_string(),
            ttl_secs: 300,
            signature: None,
        };
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: PresenceAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(ann, deserialized);
        assert!(!json.contains("signature"));
    }

    #[test]
    fn test_presence_with_signature_roundtrip() {
        let ann = PresenceAnnouncement {
            agent_id: "02abcdef".to_string(),
            name: "signed".to_string(),
            capabilities: vec![],
            waku_topic: "/test/proto".to_string(),
            ttl_secs: 60,
            signature: Some(vec![1, 2, 3, 4]),
        };
        let json = serde_json::to_string(&ann).unwrap();
        assert!(json.contains("signature"));
        let deserialized: PresenceAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.signature, Some(vec![1, 2, 3, 4]));
    }

    fn make_signing_key() -> SigningKey {
        SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng)
    }

    fn pubkey_hex(key: &SigningKey) -> String {
        hex::encode(key.verifying_key().to_encoded_point(true).as_bytes())
    }

    fn make_signed_announcement(key: &SigningKey) -> PresenceAnnouncement {
        let mut ann = PresenceAnnouncement {
            agent_id: pubkey_hex(key),
            name: "test-agent".to_string(),
            capabilities: vec!["echo".to_string(), "summarize".to_string()],
            waku_topic: "/lmao/1/task/test/proto".to_string(),
            ttl_secs: 300,
            signature: None,
        };
        ann.sign(key).unwrap();
        ann
    }

    #[test]
    fn test_sign_verify_roundtrip() {
        let key = make_signing_key();
        let ann = make_signed_announcement(&key);
        assert!(ann.signature.is_some());
        ann.verify().unwrap();
    }

    #[test]
    fn test_canonical_bytes_deterministic() {
        let key = make_signing_key();
        let ann = make_signed_announcement(&key);
        let bytes1 = ann.canonical_bytes();
        let bytes2 = ann.canonical_bytes();
        assert_eq!(bytes1, bytes2);
    }

    #[test]
    fn test_canonical_bytes_excludes_signature() {
        let key = make_signing_key();
        let ann = make_signed_announcement(&key);
        let canonical = String::from_utf8(ann.canonical_bytes()).unwrap();
        assert!(!canonical.contains("signature"));
    }

    #[test]
    fn test_tampered_name_rejected() {
        let key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        ann.name = "evil-agent".to_string();
        assert!(ann.verify().is_err());
    }

    #[test]
    fn test_tampered_capabilities_rejected() {
        let key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        ann.capabilities.push("admin".to_string());
        assert!(ann.verify().is_err());
    }

    #[test]
    fn test_tampered_agent_id_rejected() {
        let key = make_signing_key();
        let other_key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        // Swap agent_id to a different key — signature should not verify
        ann.agent_id = pubkey_hex(&other_key);
        assert!(ann.verify().is_err());
    }

    #[test]
    fn test_missing_signature_rejected() {
        let key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        ann.signature = None;
        let err = ann.verify().unwrap_err();
        assert!(err.to_string().contains("missing signature"));
    }

    #[test]
    fn test_wrong_key_signature_rejected() {
        let key = make_signing_key();
        let wrong_key = make_signing_key();
        let mut ann = PresenceAnnouncement {
            agent_id: pubkey_hex(&key),
            name: "test".to_string(),
            capabilities: vec![],
            waku_topic: "/test/proto".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        // Sign with wrong key
        ann.sign(&wrong_key).unwrap();
        assert!(ann.verify().is_err());
    }

    #[test]
    fn test_corrupted_signature_rejected() {
        let key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        // Corrupt the signature bytes
        if let Some(ref mut sig) = ann.signature {
            sig[0] ^= 0xff;
        }
        assert!(ann.verify().is_err());
    }

    #[test]
    fn test_empty_capabilities_sign_verify() {
        let key = make_signing_key();
        let mut ann = PresenceAnnouncement {
            agent_id: pubkey_hex(&key),
            name: "bare".to_string(),
            capabilities: vec![],
            waku_topic: "/test".to_string(),
            ttl_secs: 10,
            signature: None,
        };
        ann.sign(&key).unwrap();
        ann.verify().unwrap();
    }

    #[test]
    fn test_canonical_bytes_key_order() {
        let ann = PresenceAnnouncement {
            agent_id: "02ab".to_string(),
            name: "test".to_string(),
            capabilities: vec!["a".to_string()],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        let canonical = String::from_utf8(ann.canonical_bytes()).unwrap();
        // Keys should be in alphabetical order (serde_json::json! uses sorted keys)
        let agent_id_pos = canonical.find("agent_id").unwrap();
        let capabilities_pos = canonical.find("capabilities").unwrap();
        let name_pos = canonical.find("name").unwrap();
        let ttl_pos = canonical.find("ttl_secs").unwrap();
        let waku_pos = canonical.find("waku_topic").unwrap();
        assert!(agent_id_pos < capabilities_pos);
        assert!(capabilities_pos < name_pos);
        assert!(name_pos < ttl_pos);
        assert!(ttl_pos < waku_pos);
    }

    #[test]
    fn test_verify_invalid_hex_agent_id() {
        let ann = PresenceAnnouncement {
            agent_id: "not-hex!!".to_string(),
            name: "bad".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: Some(vec![1, 2, 3]),
        };
        let err = ann.verify().unwrap_err();
        assert!(err.to_string().contains("not valid hex"));
    }

    #[test]
    fn test_verify_invalid_pubkey_bytes() {
        // Valid hex but not a valid secp256k1 compressed pubkey
        let ann = PresenceAnnouncement {
            agent_id: "deadbeef".to_string(),
            name: "bad".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: Some(vec![1, 2, 3]),
        };
        let err = ann.verify().unwrap_err();
        assert!(err.to_string().contains("not a valid secp256k1"));
    }

    #[test]
    fn test_ttl_zero() {
        let key = make_signing_key();
        let mut ann = PresenceAnnouncement {
            agent_id: pubkey_hex(&key),
            name: "ephemeral".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: 0,
            signature: None,
        };
        ann.sign(&key).unwrap();
        ann.verify().unwrap();
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: PresenceAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ttl_secs, 0);
    }

    #[test]
    fn test_ttl_max() {
        let ann = PresenceAnnouncement {
            agent_id: "02ab".to_string(),
            name: "long-lived".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: u64::MAX,
            signature: None,
        };
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: PresenceAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ttl_secs, u64::MAX);
    }

    #[test]
    fn test_tampered_ttl_rejected() {
        let key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        ann.ttl_secs = 999;
        assert!(ann.verify().is_err());
    }

    #[test]
    fn test_tampered_waku_topic_rejected() {
        let key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        ann.waku_topic = "/evil/topic".to_string();
        assert!(ann.verify().is_err());
    }

    #[test]
    fn test_presence_clone_and_debug() {
        let ann = PresenceAnnouncement {
            agent_id: "02ab".to_string(),
            name: "test".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        let cloned = ann.clone();
        assert_eq!(ann, cloned);
        let debug = format!("{:?}", ann);
        assert!(debug.contains("PresenceAnnouncement"));
    }

    #[test]
    fn test_sign_sets_signature() {
        let key = make_signing_key();
        let mut ann = PresenceAnnouncement {
            agent_id: pubkey_hex(&key),
            name: "test".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        assert!(ann.signature.is_none());
        ann.sign(&key).unwrap();
        assert!(ann.signature.is_some());
        assert!(!ann.signature.as_ref().unwrap().is_empty());
    }

    #[test]
    fn test_backward_compat_json_without_signature() {
        let json = r#"{"agent_id":"02ab","name":"test","capabilities":[],"waku_topic":"/t","ttl_secs":60}"#;
        let ann: PresenceAnnouncement = serde_json::from_str(json).unwrap();
        assert!(ann.signature.is_none());
        assert_eq!(ann.name, "test");
    }

    #[test]
    fn test_resign_after_modification() {
        let key = make_signing_key();
        let mut ann = make_signed_announcement(&key);
        ann.verify().unwrap();
        // Modify and re-sign
        ann.name = "modified-agent".to_string();
        assert!(ann.verify().is_err()); // Old signature invalid
        ann.sign(&key).unwrap();
        ann.verify().unwrap(); // New signature valid
    }

    #[test]
    fn test_different_keys_produce_different_signatures() {
        let key1 = make_signing_key();
        let key2 = make_signing_key();
        let mut ann1 = PresenceAnnouncement {
            agent_id: pubkey_hex(&key1),
            name: "same".to_string(),
            capabilities: vec!["echo".to_string()],
            waku_topic: "/t".to_string(),
            ttl_secs: 300,
            signature: None,
        };
        let mut ann2 = PresenceAnnouncement {
            agent_id: pubkey_hex(&key2),
            name: "same".to_string(),
            capabilities: vec!["echo".to_string()],
            waku_topic: "/t".to_string(),
            ttl_secs: 300,
            signature: None,
        };
        ann1.sign(&key1).unwrap();
        ann2.sign(&key2).unwrap();
        assert_ne!(ann1.signature, ann2.signature);
    }

    #[test]
    fn test_unicode_name_sign_verify() {
        let key = make_signing_key();
        let mut ann = PresenceAnnouncement {
            agent_id: pubkey_hex(&key),
            name: "日本語エージェント".to_string(),
            capabilities: vec!["翻訳".to_string()],
            waku_topic: "/lmao/1/task/unicode/proto".to_string(),
            ttl_secs: 120,
            signature: None,
        };
        ann.sign(&key).unwrap();
        ann.verify().unwrap();
    }

    #[test]
    fn test_many_capabilities_sign_verify() {
        let key = make_signing_key();
        let caps: Vec<String> = (0..50).map(|i| format!("cap-{}", i)).collect();
        let mut ann = PresenceAnnouncement {
            agent_id: pubkey_hex(&key),
            name: "multi".to_string(),
            capabilities: caps,
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        ann.sign(&key).unwrap();
        ann.verify().unwrap();
    }

    #[test]
    fn test_presence_partial_eq() {
        let ann1 = PresenceAnnouncement {
            agent_id: "02ab".to_string(),
            name: "a".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        let ann2 = PresenceAnnouncement {
            agent_id: "02ab".to_string(),
            name: "b".to_string(),
            capabilities: vec![],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        assert_ne!(ann1, ann2);
        let ann3 = ann1.clone();
        assert_eq!(ann1, ann3);
    }

    #[test]
    fn test_presence_rejects_missing_required_fields() {
        let json = r#"{"agent_id":"02ab","name":"test"}"#;
        let result = serde_json::from_str::<PresenceAnnouncement>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_presence_extra_fields_ignored() {
        let json = r#"{"agent_id":"02ab","name":"test","capabilities":[],"waku_topic":"/t","ttl_secs":60,"extra":"ignored"}"#;
        let ann: PresenceAnnouncement = serde_json::from_str(json).unwrap();
        assert_eq!(ann.name, "test");
    }

    #[test]
    fn test_presence_null_signature_in_json() {
        let json = r#"{"agent_id":"02ab","name":"test","capabilities":[],"waku_topic":"/t","ttl_secs":60,"signature":null}"#;
        let ann: PresenceAnnouncement = serde_json::from_str(json).unwrap();
        assert!(ann.signature.is_none());
    }

    #[test]
    fn test_canonical_bytes_same_for_identical_announcements() {
        let ann1 = PresenceAnnouncement {
            agent_id: "02ab".to_string(),
            name: "test".to_string(),
            capabilities: vec!["a".to_string()],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: None,
        };
        let ann2 = PresenceAnnouncement {
            agent_id: "02ab".to_string(),
            name: "test".to_string(),
            capabilities: vec!["a".to_string()],
            waku_topic: "/t".to_string(),
            ttl_secs: 60,
            signature: Some(vec![1, 2, 3]), // signature should not affect canonical bytes
        };
        assert_eq!(ann1.canonical_bytes(), ann2.canonical_bytes());
    }
}
