//! Decentralized skill marketplace types.
//!
//! Skills are immutable bundles (SKILL.md + assets) stored in Logos Storage
//! (Codex) and registered on-chain via a LEZ program. Agents discover skills
//! via Messaging, rank them using a subjective trust model, and optionally
//! auto-adopt highly-rated skills.

use serde::{Deserialize, Serialize};

/// A published skill record registered in the LEZ program.
///
/// Each record links a content-addressable skill bundle (stored in Codex)
/// to on-chain metadata so other agents can discover it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRecord {
    /// Unique identifier for this skill (e.g. `"summarize-v1"`).
    pub skill_id: String,
    /// Content hash (CID) of the skill bundle in Logos Storage.
    pub content_hash: String,
    /// secp256k1 compressed public key of the skill author.
    pub author_pubkey: String,
    /// Semantic version string (e.g. `"1.0.0"`).
    pub version: String,
    /// Human-readable description of what the skill does.
    pub description: String,
    /// Searchable tags (e.g. `["nlp", "summarization"]`).
    pub tags: Vec<String>,
    /// Unix timestamp (seconds) when the skill was published.
    pub timestamp: u64,
}

/// A rating submitted by an agent for a skill.
///
/// Ratings are public and on-chain. A small stake may be required to
/// prevent spam (enforced by the LEZ program, not this struct).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRating {
    /// The skill being rated.
    pub skill_id: String,
    /// Public key of the agent submitting the rating.
    pub rater_pubkey: String,
    /// Score from 1 (poor) to 5 (excellent).
    pub score: u8,
    /// Optional freeform review text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Unix timestamp (seconds) when the rating was submitted.
    pub timestamp: u64,
}

/// An agent's trust set — the list of peer pubkeys whose skill ratings
/// this agent weighs when computing effective rank.
///
/// Trust is subjective: different agents have different trust sets,
/// leading to different installed skill sets (no monoculture).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustSet {
    /// Direct trust entries: `(pubkey, weight)` where weight is 0.0..=1.0.
    pub entries: Vec<TrustEntry>,
    /// Whether to transitively trust peers of trusted peers.
    pub transitive: bool,
    /// Decay factor for transitive trust (e.g. 0.5 = half weight per hop).
    /// Ignored if `transitive` is false.
    pub transitive_decay: f64,
}

/// A single entry in an agent's trust set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustEntry {
    /// Public key of the trusted peer.
    pub pubkey: String,
    /// Weight assigned to this peer's ratings (0.0..=1.0).
    pub weight: f64,
}

/// Configuration for the auto-adopt behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoAdoptConfig {
    /// Minimum effective rank (weighted average from trust set) to trigger
    /// auto-adoption. Range: 1.0..=5.0.
    pub threshold: f64,
    /// Minimum number of trusted ratings before considering auto-adopt.
    pub min_ratings: usize,
    /// Whether to tip the skill author on auto-adopt via payment infra.
    pub tip_on_adopt: bool,
    /// Tip amount in token units (only used if `tip_on_adopt` is true).
    pub tip_amount: u64,
}

impl Default for AutoAdoptConfig {
    fn default() -> Self {
        Self {
            threshold: 4.0,
            min_ratings: 2,
            tip_on_adopt: false,
            tip_amount: 0,
        }
    }
}

impl Default for TrustSet {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            transitive: false,
            transitive_decay: 0.5,
        }
    }
}

impl TrustSet {
    /// Look up the trust weight for a given pubkey. Returns 0.0 if not trusted.
    pub fn weight_of(&self, pubkey: &str) -> f64 {
        self.entries
            .iter()
            .find(|e| e.pubkey == pubkey)
            .map(|e| e.weight)
            .unwrap_or(0.0)
    }

    /// Compute the effective rank of a skill from a set of ratings,
    /// weighted by this trust set.
    ///
    /// Returns `None` if no trusted ratings exist.
    pub fn effective_rank(&self, ratings: &[SkillRating]) -> Option<f64> {
        let mut weighted_sum = 0.0;
        let mut weight_total = 0.0;

        for rating in ratings {
            let w = self.weight_of(&rating.rater_pubkey);
            if w > 0.0 {
                weighted_sum += w * rating.score as f64;
                weight_total += w;
            }
        }

        if weight_total > 0.0 {
            Some(weighted_sum / weight_total)
        } else {
            None
        }
    }
}

/// Feedback message sent from a skill consumer to the author via Messaging.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillFeedback {
    /// The skill being reviewed.
    pub skill_id: String,
    /// Public key of the feedback sender.
    pub sender_pubkey: String,
    /// Structured feedback type.
    pub kind: FeedbackKind,
    /// Freeform feedback text.
    pub message: String,
    /// Unix timestamp.
    pub timestamp: u64,
}

/// Structured feedback categories for skill improvement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    /// The skill worked well — positive signal.
    Positive,
    /// Bug report or failure case.
    BugReport,
    /// Feature request or improvement suggestion.
    FeatureRequest,
    /// General comment.
    General,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> SkillRecord {
        SkillRecord {
            skill_id: "summarize-v1".into(),
            content_hash: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".into(),
            author_pubkey: "02abcdef".into(),
            version: "1.0.0".into(),
            description: "Summarizes text".into(),
            tags: vec!["nlp".into(), "summarization".into()],
            timestamp: 1713200000,
        }
    }

    fn sample_rating(rater: &str, score: u8) -> SkillRating {
        SkillRating {
            skill_id: "summarize-v1".into(),
            rater_pubkey: rater.into(),
            score,
            comment: None,
            timestamp: 1713200100,
        }
    }

    #[test]
    fn skill_record_serialization_roundtrip() {
        let record = sample_record();
        let json = serde_json::to_string(&record).unwrap();
        let deserialized: SkillRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, deserialized);
    }

    #[test]
    fn skill_rating_serialization_roundtrip() {
        let rating = SkillRating {
            skill_id: "summarize-v1".into(),
            rater_pubkey: "02abcdef".into(),
            score: 4,
            comment: Some("Works great!".into()),
            timestamp: 1713200100,
        };
        let json = serde_json::to_string(&rating).unwrap();
        let deserialized: SkillRating = serde_json::from_str(&json).unwrap();
        assert_eq!(rating, deserialized);
    }

    #[test]
    fn skill_rating_without_comment_omits_field() {
        let rating = sample_rating("02ab", 5);
        let json = serde_json::to_string(&rating).unwrap();
        assert!(!json.contains("comment"));
    }

    #[test]
    fn trust_set_weight_of_known_peer() {
        let ts = TrustSet {
            entries: vec![
                TrustEntry {
                    pubkey: "peer-a".into(),
                    weight: 0.8,
                },
                TrustEntry {
                    pubkey: "peer-b".into(),
                    weight: 0.5,
                },
            ],
            transitive: false,
            transitive_decay: 0.5,
        };
        assert!((ts.weight_of("peer-a") - 0.8).abs() < f64::EPSILON);
        assert!((ts.weight_of("peer-b") - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn trust_set_weight_of_unknown_peer_is_zero() {
        let ts = TrustSet::default();
        assert!((ts.weight_of("unknown") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn effective_rank_weighted_average() {
        let ts = TrustSet {
            entries: vec![
                TrustEntry {
                    pubkey: "peer-a".into(),
                    weight: 1.0,
                },
                TrustEntry {
                    pubkey: "peer-b".into(),
                    weight: 0.5,
                },
            ],
            transitive: false,
            transitive_decay: 0.5,
        };
        let ratings = vec![
            sample_rating("peer-a", 5), // weight 1.0
            sample_rating("peer-b", 3), // weight 0.5
            sample_rating("peer-c", 1), // untrusted, ignored
        ];
        let rank = ts.effective_rank(&ratings).unwrap();
        // (1.0*5 + 0.5*3) / (1.0 + 0.5) = 6.5 / 1.5 = 4.333...
        assert!((rank - 4.333333333333333).abs() < 1e-10);
    }

    #[test]
    fn effective_rank_no_trusted_ratings_returns_none() {
        let ts = TrustSet {
            entries: vec![TrustEntry {
                pubkey: "peer-a".into(),
                weight: 1.0,
            }],
            transitive: false,
            transitive_decay: 0.5,
        };
        let ratings = vec![sample_rating("unknown-peer", 5)];
        assert!(ts.effective_rank(&ratings).is_none());
    }

    #[test]
    fn effective_rank_empty_ratings_returns_none() {
        let ts = TrustSet {
            entries: vec![TrustEntry {
                pubkey: "peer-a".into(),
                weight: 1.0,
            }],
            transitive: false,
            transitive_decay: 0.5,
        };
        assert!(ts.effective_rank(&[]).is_none());
    }

    #[test]
    fn auto_adopt_config_default() {
        let config = AutoAdoptConfig::default();
        assert!((config.threshold - 4.0).abs() < f64::EPSILON);
        assert_eq!(config.min_ratings, 2);
        assert!(!config.tip_on_adopt);
        assert_eq!(config.tip_amount, 0);
    }

    #[test]
    fn auto_adopt_config_serialization_roundtrip() {
        let config = AutoAdoptConfig {
            threshold: 3.5,
            min_ratings: 5,
            tip_on_adopt: true,
            tip_amount: 100,
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AutoAdoptConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn trust_set_serialization_roundtrip() {
        let ts = TrustSet {
            entries: vec![TrustEntry {
                pubkey: "peer-a".into(),
                weight: 0.9,
            }],
            transitive: true,
            transitive_decay: 0.3,
        };
        let json = serde_json::to_string(&ts).unwrap();
        let deserialized: TrustSet = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, deserialized);
    }

    #[test]
    fn feedback_kind_serialization() {
        let fb = SkillFeedback {
            skill_id: "summarize-v1".into(),
            sender_pubkey: "02ab".into(),
            kind: FeedbackKind::BugReport,
            message: "Crashes on empty input".into(),
            timestamp: 1713200200,
        };
        let json = serde_json::to_string(&fb).unwrap();
        assert!(json.contains("bug_report"));
        let deserialized: SkillFeedback = serde_json::from_str(&json).unwrap();
        assert_eq!(fb, deserialized);
    }

    #[test]
    fn all_feedback_kinds_serialize() {
        let kinds = vec![
            FeedbackKind::Positive,
            FeedbackKind::BugReport,
            FeedbackKind::FeatureRequest,
            FeedbackKind::General,
        ];
        for kind in kinds {
            let fb = SkillFeedback {
                skill_id: "s".into(),
                sender_pubkey: "p".into(),
                kind: kind.clone(),
                message: "m".into(),
                timestamp: 0,
            };
            let json = serde_json::to_string(&fb).unwrap();
            let deserialized: SkillFeedback = serde_json::from_str(&json).unwrap();
            assert_eq!(fb, deserialized);
        }
    }

    #[test]
    fn skill_record_clone_and_debug() {
        let record = sample_record();
        let cloned = record.clone();
        assert_eq!(record, cloned);
        let debug = format!("{:?}", record);
        assert!(debug.contains("summarize-v1"));
    }

    #[test]
    fn effective_rank_single_trusted_rating() {
        let ts = TrustSet {
            entries: vec![TrustEntry {
                pubkey: "peer-a".into(),
                weight: 0.7,
            }],
            transitive: false,
            transitive_decay: 0.5,
        };
        let ratings = vec![sample_rating("peer-a", 4)];
        let rank = ts.effective_rank(&ratings).unwrap();
        assert!((rank - 4.0).abs() < f64::EPSILON);
    }
}
