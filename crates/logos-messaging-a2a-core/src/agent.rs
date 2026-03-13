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

    #[test]
    fn test_agent_card_empty_strings() {
        let card = AgentCard {
            name: "".to_string(),
            description: "".to_string(),
            version: "".to_string(),
            capabilities: vec![],
            public_key: "".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, deserialized);
    }

    #[test]
    fn test_agent_card_many_capabilities() {
        let caps: Vec<String> = (0..100).map(|i| format!("cap-{}", i)).collect();
        let card = AgentCard {
            name: "multi".to_string(),
            description: "lots of caps".to_string(),
            version: "1.0.0".to_string(),
            capabilities: caps.clone(),
            public_key: "02ff".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.capabilities.len(), 100);
        assert_eq!(deserialized.capabilities, caps);
    }

    #[test]
    fn test_agent_card_clone() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "Echoes".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02abcdef".to_string(),
            intro_bundle: None,
        };
        let cloned = card.clone();
        assert_eq!(card, cloned);
    }

    #[test]
    fn test_agent_card_debug() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "Echoes".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let debug = format!("{:?}", card);
        assert!(debug.contains("echo"));
        assert!(debug.contains("AgentCard"));
    }

    #[test]
    fn test_agent_card_partial_eq() {
        let card1 = AgentCard {
            name: "a".to_string(),
            description: "d".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let card2 = AgentCard {
            name: "b".to_string(),
            description: "d".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        assert_ne!(card1, card2);
    }

    #[test]
    fn test_agent_card_json_field_names() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "desc".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["cap".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("name").is_some());
        assert!(v.get("description").is_some());
        assert!(v.get("version").is_some());
        assert!(v.get("capabilities").is_some());
        assert!(v.get("public_key").is_some());
        assert!(v.get("intro_bundle").is_none()); // skipped when None
    }

    #[test]
    fn test_agent_card_deserialization_with_null_intro_bundle() {
        let json = r#"{"name":"echo","description":"d","version":"0.1.0","capabilities":[],"public_key":"02ab","intro_bundle":null}"#;
        let card: AgentCard = serde_json::from_str(json).unwrap();
        assert!(card.intro_bundle.is_none());
    }

    #[test]
    fn test_agent_card_unicode_strings() {
        let card = AgentCard {
            name: "エージェント".to_string(),
            description: "日本語の説明".to_string(),
            version: "1.0.0".to_string(),
            capabilities: vec!["翻訳".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, deserialized);
    }

    #[test]
    fn test_agent_card_special_characters_in_strings() {
        let card = AgentCard {
            name: r#"agent "quoted""#.to_string(),
            description: "line1\nline2".to_string(),
            version: "0.1.0-beta+build.123".to_string(),
            capabilities: vec!["cap/with/slashes".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, deserialized);
    }

    #[test]
    fn test_agent_card_deserialization_rejects_missing_fields() {
        let json = r#"{"name":"echo","description":"d"}"#;
        let result = serde_json::from_str::<AgentCard>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_card_extra_fields_ignored() {
        let json = r#"{"name":"echo","description":"d","version":"0.1.0","capabilities":[],"public_key":"02ab","unknown_field":"should be ignored"}"#;
        let card: AgentCard = serde_json::from_str(json).unwrap();
        assert_eq!(card.name, "echo");
    }

    #[test]
    fn test_agent_card_capabilities_order_preserved() {
        let card = AgentCard {
            name: "order".to_string(),
            description: "d".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["z".to_string(), "a".to_string(), "m".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.capabilities, vec!["z", "a", "m"]);
    }

    #[test]
    fn test_agent_card_duplicate_capabilities() {
        let card = AgentCard {
            name: "dup".to_string(),
            description: "d".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["echo".to_string(), "echo".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.capabilities.len(), 2);
    }

    #[test]
    fn test_agent_card_rejects_invalid_json() {
        let result = serde_json::from_str::<AgentCard>("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_card_rejects_wrong_type_for_capabilities() {
        let json = r#"{"name":"echo","description":"d","version":"0.1.0","capabilities":"not_an_array","public_key":"02ab"}"#;
        let result = serde_json::from_str::<AgentCard>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_card_serde_json_value_roundtrip() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "desc".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["cap".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let value = serde_json::to_value(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_value(value).unwrap();
        assert_eq!(card, deserialized);
    }

    #[test]
    fn test_agent_card_pretty_json_roundtrip() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "desc".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["cap".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: None,
        };
        let pretty = serde_json::to_string_pretty(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&pretty).unwrap();
        assert_eq!(card, deserialized);
    }
}
