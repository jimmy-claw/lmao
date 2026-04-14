//! Logos Messaging content topic for agent intro bundle discovery.
//!
//! Instead of sharing intro bundles out-of-band, agents broadcast them on a
//! well-known Logos Messaging content topic. This allows agent discovery and
//! encrypted session establishment to happen entirely over the decentralized
//! transport layer.

use logos_messaging_a2a_core::AgentCard;
use logos_messaging_a2a_crypto::IntroBundle;
use serde::{Deserialize, Serialize};

/// Well-known content topic where agents broadcast their intro bundles.
///
/// Agents subscribe to this topic on startup to discover peers' encryption
/// keys, then establish pairwise [`ChatSession`](crate::ChatSession)s.
pub const INTRO_BUNDLE_DISCOVERY: &str = "/lmao/1/intro-bundle/proto";

/// Returns a content topic scoped to a specific agent's intro bundle.
///
/// Useful when an agent wants to request or refresh a specific peer's bundle
/// rather than listening to the broadcast topic.
pub fn intro_bundle_topic(agent_pubkey: &str) -> String {
    format!("/lmao/1/intro-bundle/{}/proto", agent_pubkey)
}

/// An intro bundle announcement broadcast on the discovery topic.
///
/// Combines the agent's [`AgentCard`] with its [`IntroBundle`] so that
/// discovering agents get both identity and encryption material in one message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntroBundleAnnouncement {
    /// The agent's full card (name, capabilities, signing pubkey).
    pub agent_card: AgentCard,
    /// The agent's X25519 intro bundle for session establishment.
    pub intro_bundle: IntroBundle,
}

impl IntroBundleAnnouncement {
    /// Create an announcement from an agent card.
    ///
    /// Returns `None` if the agent card has no intro bundle (encryption not enabled).
    pub fn from_card(card: &AgentCard) -> Option<Self> {
        card.intro_bundle.as_ref().map(|bundle| Self {
            agent_card: card.clone(),
            intro_bundle: bundle.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_topic_is_well_formed() {
        assert!(INTRO_BUNDLE_DISCOVERY.starts_with('/'));
        assert!(INTRO_BUNDLE_DISCOVERY.ends_with("/proto"));
        assert!(INTRO_BUNDLE_DISCOVERY.contains("intro-bundle"));
    }

    #[test]
    fn agent_specific_topic() {
        let topic = intro_bundle_topic("02abcdef");
        assert_eq!(topic, "/lmao/1/intro-bundle/02abcdef/proto");
    }

    #[test]
    fn agent_specific_topic_empty_key() {
        let topic = intro_bundle_topic("");
        assert_eq!(topic, "/lmao/1/intro-bundle//proto");
    }

    #[test]
    fn announcement_from_card_with_bundle() {
        let card = AgentCard {
            name: "agent-a".to_string(),
            description: "Test agent".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02abcdef".to_string(),
            intro_bundle: Some(IntroBundle::new("aabbccdd")),
        };
        let ann = IntroBundleAnnouncement::from_card(&card).unwrap();
        assert_eq!(ann.agent_card.name, "agent-a");
        assert_eq!(ann.intro_bundle.agent_pubkey, "aabbccdd");
    }

    #[test]
    fn announcement_from_card_without_bundle_is_none() {
        let card = AgentCard {
            name: "plain-agent".to_string(),
            description: "No encryption".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![],
            public_key: "02abcdef".to_string(),
            intro_bundle: None,
        };
        assert!(IntroBundleAnnouncement::from_card(&card).is_none());
    }

    #[test]
    fn announcement_serialization_roundtrip() {
        let card = AgentCard {
            name: "agent-a".to_string(),
            description: "Test".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02ab".to_string(),
            intro_bundle: Some(IntroBundle::new("aabb")),
        };
        let ann = IntroBundleAnnouncement::from_card(&card).unwrap();
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: IntroBundleAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(ann, deserialized);
    }

    #[test]
    fn discovery_and_agent_topics_are_distinct() {
        let agent_topic = intro_bundle_topic("02abcdef");
        assert_ne!(INTRO_BUNDLE_DISCOVERY, agent_topic);
    }
}
