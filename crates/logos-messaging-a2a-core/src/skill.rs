//! Skill marketplace types for the decentralized skill sharing system.
//!
//! Agents publish skill bundles to Logos Storage, register them in a LEZ
//! program, discover them via Messaging, rate them on-chain, and
//! automatically adopt highly-ranked skills from trusted peers.

use serde::{Deserialize, Serialize};

/// Metadata describing a published skill bundle.
///
/// A skill is identified by its `skill_id` (unique name) and pinned to a
/// specific `content_hash` (Logos Storage / Codex CID) for immutable,
/// verifiable retrieval.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillDescriptor {
    /// Unique identifier for this skill (e.g. `"summarize-long-docs"`).
    pub skill_id: String,
    /// Content hash (CID) of the skill bundle in Logos Storage.
    pub content_hash: String,
    /// secp256k1 compressed public key of the skill author.
    pub author_pubkey: String,
    /// Semantic version of this skill (e.g. `"1.0.0"`).
    pub version: String,
    /// Human-readable description of what this skill does.
    pub description: String,
    /// Searchable tags (e.g. `["nlp", "summarization"]`).
    pub tags: Vec<String>,
    /// Unix timestamp (seconds) when this version was published.
    pub timestamp: u64,
}

/// A rating submitted by an agent for a skill.
///
/// Ratings are public and on-chain. Each agent can rate a skill once
/// (latest rating wins). A small stake prevents spam.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRating {
    /// The skill being rated.
    pub skill_id: String,
    /// Public key of the agent submitting this rating.
    pub rater_pubkey: String,
    /// Rating score from 1 (poor) to 5 (excellent).
    pub score: u8,
    /// Optional freeform comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Unix timestamp (seconds) of this rating.
    pub timestamp: u64,
}

/// Announcement broadcast on the skill discovery topic when a skill is
/// published or updated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillAnnouncement {
    /// The skill descriptor being announced.
    pub skill: SkillDescriptor,
    /// Whether this is a new skill or an update to an existing one.
    pub is_update: bool,
}

impl SkillRating {
    /// Validate that the score is in the allowed range (1..=5).
    pub fn is_valid(&self) -> bool {
        (1..=5).contains(&self.score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_descriptor() -> SkillDescriptor {
        SkillDescriptor {
            skill_id: "summarize-docs".into(),
            content_hash: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".into(),
            author_pubkey: "02abcdef1234567890".into(),
            version: "1.0.0".into(),
            description: "Summarizes long documents".into(),
            tags: vec!["nlp".into(), "summarization".into()],
            timestamp: 1713360000,
        }
    }

    #[test]
    fn skill_descriptor_serialization_roundtrip() {
        let desc = sample_descriptor();
        let json = serde_json::to_string(&desc).unwrap();
        let deserialized: SkillDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(desc, deserialized);
    }

    #[test]
    fn skill_rating_valid_scores() {
        for score in 1..=5 {
            let rating = SkillRating {
                skill_id: "test".into(),
                rater_pubkey: "02ab".into(),
                score,
                comment: None,
                timestamp: 0,
            };
            assert!(rating.is_valid());
        }
    }

    #[test]
    fn skill_rating_invalid_scores() {
        for score in [0, 6, 255] {
            let rating = SkillRating {
                skill_id: "test".into(),
                rater_pubkey: "02ab".into(),
                score,
                comment: None,
                timestamp: 0,
            };
            assert!(!rating.is_valid());
        }
    }

    #[test]
    fn skill_rating_comment_skipped_when_none() {
        let rating = SkillRating {
            skill_id: "test".into(),
            rater_pubkey: "02ab".into(),
            score: 4,
            comment: None,
            timestamp: 100,
        };
        let json = serde_json::to_string(&rating).unwrap();
        assert!(!json.contains("comment"));
    }

    #[test]
    fn skill_announcement_serialization() {
        let ann = SkillAnnouncement {
            skill: sample_descriptor(),
            is_update: false,
        };
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: SkillAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(ann, deserialized);
        assert!(!deserialized.is_update);
    }

    #[test]
    fn skill_announcement_update() {
        let ann = SkillAnnouncement {
            skill: sample_descriptor(),
            is_update: true,
        };
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: SkillAnnouncement = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_update);
    }
}
