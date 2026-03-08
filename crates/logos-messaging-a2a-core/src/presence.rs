use serde::{Deserialize, Serialize};

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
}
