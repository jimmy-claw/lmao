use logos_messaging_a2a_crypto::IntroBundle;
use serde::{Deserialize, Serialize};

/// Agent identity and capability advertisement.
/// Equivalent to A2A's AgentCard — broadcast on the discovery topic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCard {
    /// Human-readable display name of the agent (e.g. `"echo-agent"`).
    pub name: String,
    /// Short summary of what this agent does, suitable for UI display.
    pub description: String,
    /// Semantic version of the agent implementation (e.g. `"0.1.0"`).
    pub version: String,
    /// Capability tags this agent advertises (e.g. `["summarize", "translate"]`).
    /// Clients use these to discover agents that support a specific skill.
    pub capabilities: Vec<String>,
    /// secp256k1 compressed public key as hex string — agent identity
    pub public_key: String,
    /// X25519 intro bundle for encrypted sessions (None = no encryption)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro_bundle: Option<IntroBundle>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_card_serialization() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "Echoes messages".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02abcdef".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, deserialized);
        // intro_bundle should be absent when None
        assert!(!json.contains("intro_bundle"));
    }

    #[test]
    fn test_agent_card_with_intro_bundle() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "Echoes messages".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02abcdef".to_string(),
            intro_bundle: Some(IntroBundle::new("aabbccdd")),
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, deserialized);
        assert!(json.contains("intro_bundle"));
    }

    #[test]
    fn test_backward_compat_agent_card_without_intro_bundle() {
        // JSON without intro_bundle field should deserialize fine (defaults to None)
        let json = r#"{"name":"echo","description":"Echoes","version":"0.1.0","capabilities":["text"],"public_key":"02abcdef"}"#;
        let card: AgentCard = serde_json::from_str(json).unwrap();
        assert_eq!(card.name, "echo");
        assert!(card.intro_bundle.is_none());
    }

    #[test]
    fn test_agent_card_empty_capabilities() {
        let card = AgentCard {
            name: "bare".to_string(),
            description: "no caps".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![],
            public_key: "02dead".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert!(deserialized.capabilities.is_empty());
    }
}
