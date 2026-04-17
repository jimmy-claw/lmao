//! Decentralized skill marketplace — publish, discover, rank, auto-adopt.
//!
//! This module adds skill marketplace capabilities to [`WakuA2ANode`]:
//!
//! - **Publish**: upload a skill bundle to Logos Storage and announce it.
//! - **Discover**: subscribe to skill announcements and query the registry.
//! - **Rank**: compute subjective skill rankings using a per-agent trust set.
//! - **Auto-adopt**: monitor rankings and adopt skills that cross a threshold.

use logos_messaging_a2a_core::skill::{SkillAnnouncement, SkillDescriptor, SkillRating};
use logos_messaging_a2a_core::skill_registry::SkillRegistry;
use logos_messaging_a2a_core::{topics, A2AEnvelope};
use logos_messaging_a2a_transport::Transport;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::metrics::Metrics;
use crate::{NodeError, Result, WakuA2ANode};

/// Configuration for the skill marketplace on a node.
pub struct SkillMarketplaceConfig {
    /// Minimum effective rank (1.0–5.0) for auto-adoption.
    pub auto_adopt_threshold: f64,
    /// Whether transitive trust is enabled ("trust friends of friends").
    pub transitive_trust: bool,
    /// Decay weight for transitive trust (0.0–1.0). Applied multiplicatively
    /// for each hop.
    pub transitive_decay: f64,
}

impl Default for SkillMarketplaceConfig {
    fn default() -> Self {
        Self {
            auto_adopt_threshold: 4.0,
            transitive_trust: false,
            transitive_decay: 0.5,
        }
    }
}

/// Per-agent skill marketplace state.
///
/// Tracks the trust set, known skills, and locally installed skills.
/// Ranking is subjective — each agent computes its own effective rank
/// based only on ratings from trusted peers.
pub struct SkillMarketplace {
    /// Agent pubkeys whose ratings this agent trusts.
    trust_set: std::sync::RwLock<HashSet<String>>,
    /// Known skill descriptors, keyed by skill_id.
    known_skills: std::sync::RwLock<HashMap<String, SkillDescriptor>>,
    /// Locally installed skill IDs.
    installed_skills: std::sync::RwLock<HashSet<String>>,
    /// Cached ratings keyed by skill_id.
    cached_ratings: std::sync::RwLock<HashMap<String, Vec<SkillRating>>>,
    /// Optional skill registry (LEZ on-chain).
    registry: Option<Arc<dyn SkillRegistry>>,
    /// Marketplace configuration.
    pub config: SkillMarketplaceConfig,
}

impl SkillMarketplace {
    /// Create a new skill marketplace with default configuration.
    pub fn new() -> Self {
        Self {
            trust_set: std::sync::RwLock::new(HashSet::new()),
            known_skills: std::sync::RwLock::new(HashMap::new()),
            installed_skills: std::sync::RwLock::new(HashSet::new()),
            cached_ratings: std::sync::RwLock::new(HashMap::new()),
            registry: None,
            config: SkillMarketplaceConfig::default(),
        }
    }

    /// Create with a skill registry backend.
    pub fn with_registry(mut self, registry: Arc<dyn SkillRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Create with custom configuration.
    pub fn with_config(mut self, config: SkillMarketplaceConfig) -> Self {
        self.config = config;
        self
    }

    // ── Trust set management ───────────────────────────────────────

    /// Add an agent to the trust set.
    pub fn trust(&self, pubkey: &str) {
        self.trust_set.write().unwrap().insert(pubkey.to_string());
    }

    /// Remove an agent from the trust set.
    pub fn untrust(&self, pubkey: &str) {
        self.trust_set.write().unwrap().remove(pubkey);
    }

    /// Check if an agent is trusted.
    pub fn is_trusted(&self, pubkey: &str) -> bool {
        self.trust_set.read().unwrap().contains(pubkey)
    }

    /// Get the current trust set.
    pub fn trust_set(&self) -> HashSet<String> {
        self.trust_set.read().unwrap().clone()
    }

    // ── Skill tracking ─────────────────────────────────────────────

    /// Record a discovered skill.
    pub fn track_skill(&self, skill: SkillDescriptor) {
        self.known_skills
            .write()
            .unwrap()
            .insert(skill.skill_id.clone(), skill);
    }

    /// Get a known skill by ID.
    pub fn get_skill(&self, skill_id: &str) -> Option<SkillDescriptor> {
        self.known_skills.read().unwrap().get(skill_id).cloned()
    }

    /// List all known skills.
    pub fn known_skills(&self) -> Vec<SkillDescriptor> {
        self.known_skills.read().unwrap().values().cloned().collect()
    }

    /// Mark a skill as locally installed.
    pub fn mark_installed(&self, skill_id: &str) {
        self.installed_skills
            .write()
            .unwrap()
            .insert(skill_id.to_string());
    }

    /// Check if a skill is installed.
    pub fn is_installed(&self, skill_id: &str) -> bool {
        self.installed_skills.read().unwrap().contains(skill_id)
    }

    // ── Rating & ranking ───────────────────────────────────────────

    /// Cache ratings for a skill.
    pub fn cache_ratings(&self, skill_id: &str, ratings: Vec<SkillRating>) {
        self.cached_ratings
            .write()
            .unwrap()
            .insert(skill_id.to_string(), ratings);
    }

    /// Compute the effective rank of a skill using only trusted ratings.
    ///
    /// Returns `None` if no trusted ratings exist.
    pub fn effective_rank(&self, skill_id: &str) -> Option<f64> {
        let ratings = self.cached_ratings.read().unwrap();
        let trust = self.trust_set.read().unwrap();

        let trusted_ratings: Vec<&SkillRating> = ratings
            .get(skill_id)
            .map(|rs| {
                rs.iter()
                    .filter(|r| trust.contains(&r.rater_pubkey))
                    .collect()
            })
            .unwrap_or_default();

        if trusted_ratings.is_empty() {
            return None;
        }

        let sum: f64 = trusted_ratings.iter().map(|r| r.score as f64).sum();
        Some(sum / trusted_ratings.len() as f64)
    }

    /// Return skill IDs whose effective rank meets or exceeds the
    /// auto-adopt threshold and are not yet installed.
    pub fn adoption_candidates(&self) -> Vec<String> {
        let known = self.known_skills.read().unwrap();
        let installed = self.installed_skills.read().unwrap();
        let threshold = self.config.auto_adopt_threshold;

        known
            .keys()
            .filter(|id| !installed.contains(id.as_str()))
            .filter(|id| {
                self.effective_rank(id)
                    .map(|r| r >= threshold)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// Access the optional skill registry.
    pub fn registry(&self) -> Option<&Arc<dyn SkillRegistry>> {
        self.registry.as_ref()
    }
}

impl Default for SkillMarketplace {
    fn default() -> Self {
        Self::new()
    }
}

// ── Node integration ───────────────────────────────────────────────

impl<T: Transport> WakuA2ANode<T> {
    /// Publish a skill announcement on the skills topic.
    ///
    /// Broadcasts the skill descriptor so subscribed agents can discover it.
    pub async fn announce_skill(&self, skill: SkillDescriptor, is_update: bool) -> Result<()> {
        let announcement = SkillAnnouncement { skill, is_update };
        let envelope = A2AEnvelope::SkillAnnouncement(announcement);
        let payload = serde_json::to_vec(&envelope)?;
        self.channel
            .transport()
            .publish(topics::SKILLS, &payload)
            .await?;
        Metrics::inc(&self.metrics.messages_published);
        tracing::info!(pubkey = %self.pubkey(), "Skill announced");
        Ok(())
    }

    /// Discover skills by subscribing to the skills topic and draining messages.
    ///
    /// Returns all skill announcements received since the last call.
    pub async fn discover_skills(&self) -> Result<Vec<SkillAnnouncement>> {
        let mut rx = self
            .channel
            .transport()
            .subscribe(topics::SKILLS)
            .await?;

        let mut announcements = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Ok(A2AEnvelope::SkillAnnouncement(ann)) = serde_json::from_slice(&msg) {
                announcements.push(ann);
            }
        }

        let _ = self
            .channel
            .transport()
            .unsubscribe(topics::SKILLS)
            .await;
        Ok(announcements)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_messaging_a2a_core::skill::{SkillDescriptor, SkillRating};
    use logos_messaging_a2a_core::skill_registry::InMemorySkillRegistry;

    fn sample_skill(id: &str) -> SkillDescriptor {
        SkillDescriptor {
            skill_id: id.into(),
            content_hash: "bafytest".into(),
            author_pubkey: "02author".into(),
            version: "1.0.0".into(),
            description: format!("{} skill", id),
            tags: vec!["test".into()],
            timestamp: 1000,
        }
    }

    fn rating(skill_id: &str, rater: &str, score: u8) -> SkillRating {
        SkillRating {
            skill_id: skill_id.into(),
            rater_pubkey: rater.into(),
            score,
            comment: None,
            timestamp: 2000,
        }
    }

    #[test]
    fn trust_set_management() {
        let mp = SkillMarketplace::new();
        assert!(!mp.is_trusted("02aa"));
        mp.trust("02aa");
        assert!(mp.is_trusted("02aa"));
        mp.untrust("02aa");
        assert!(!mp.is_trusted("02aa"));
    }

    #[test]
    fn track_and_get_skill() {
        let mp = SkillMarketplace::new();
        let skill = sample_skill("sum");
        mp.track_skill(skill.clone());
        assert_eq!(mp.get_skill("sum"), Some(skill));
        assert_eq!(mp.get_skill("missing"), None);
    }

    #[test]
    fn known_skills_lists_all() {
        let mp = SkillMarketplace::new();
        mp.track_skill(sample_skill("a"));
        mp.track_skill(sample_skill("b"));
        assert_eq!(mp.known_skills().len(), 2);
    }

    #[test]
    fn installed_tracking() {
        let mp = SkillMarketplace::new();
        assert!(!mp.is_installed("sum"));
        mp.mark_installed("sum");
        assert!(mp.is_installed("sum"));
    }

    #[test]
    fn effective_rank_no_ratings() {
        let mp = SkillMarketplace::new();
        assert_eq!(mp.effective_rank("sum"), None);
    }

    #[test]
    fn effective_rank_no_trusted_ratings() {
        let mp = SkillMarketplace::new();
        mp.cache_ratings("sum", vec![rating("sum", "02aa", 5)]);
        // 02aa is not in trust set
        assert_eq!(mp.effective_rank("sum"), None);
    }

    #[test]
    fn effective_rank_with_trusted_ratings() {
        let mp = SkillMarketplace::new();
        mp.trust("02aa");
        mp.trust("02bb");
        mp.cache_ratings(
            "sum",
            vec![
                rating("sum", "02aa", 5),
                rating("sum", "02bb", 3),
                rating("sum", "02cc", 1), // untrusted, ignored
            ],
        );
        let rank = mp.effective_rank("sum").unwrap();
        assert!((rank - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn adoption_candidates_above_threshold() {
        let mp = SkillMarketplace::new();
        mp.trust("02aa");
        mp.track_skill(sample_skill("good"));
        mp.track_skill(sample_skill("bad"));
        mp.track_skill(sample_skill("installed"));

        mp.cache_ratings("good", vec![rating("good", "02aa", 5)]);
        mp.cache_ratings("bad", vec![rating("bad", "02aa", 2)]);
        mp.cache_ratings("installed", vec![rating("installed", "02aa", 5)]);
        mp.mark_installed("installed");

        let candidates = mp.adoption_candidates();
        assert_eq!(candidates, vec!["good".to_string()]);
    }

    #[test]
    fn adoption_candidates_empty_when_all_installed() {
        let mp = SkillMarketplace::new();
        mp.trust("02aa");
        mp.track_skill(sample_skill("sum"));
        mp.cache_ratings("sum", vec![rating("sum", "02aa", 5)]);
        mp.mark_installed("sum");

        assert!(mp.adoption_candidates().is_empty());
    }

    #[test]
    fn default_config() {
        let config = SkillMarketplaceConfig::default();
        assert!((config.auto_adopt_threshold - 4.0).abs() < f64::EPSILON);
        assert!(!config.transitive_trust);
        assert!((config.transitive_decay - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn with_registry() {
        let registry = Arc::new(InMemorySkillRegistry::new());
        let mp = SkillMarketplace::new().with_registry(registry);
        assert!(mp.registry().is_some());
    }

    #[test]
    fn with_config() {
        let config = SkillMarketplaceConfig {
            auto_adopt_threshold: 3.0,
            transitive_trust: true,
            transitive_decay: 0.8,
        };
        let mp = SkillMarketplace::new().with_config(config);
        assert!((mp.config.auto_adopt_threshold - 3.0).abs() < f64::EPSILON);
        assert!(mp.config.transitive_trust);
    }

    #[tokio::test]
    async fn announce_and_discover_skills() {
        use logos_messaging_a2a_transport::memory::InMemoryTransport;

        let transport = InMemoryTransport::new();
        let node_a = WakuA2ANode::new("a", "agent a", vec![], transport.clone());
        let node_b = WakuA2ANode::new("b", "agent b", vec![], transport.clone());

        // B subscribes first, then A announces
        let mut rx = transport.subscribe(topics::SKILLS).await.unwrap();
        let skill = sample_skill("test-skill");
        node_a.announce_skill(skill.clone(), false).await.unwrap();

        // Verify the message was published
        let msg = rx.try_recv().unwrap();
        let envelope: A2AEnvelope = serde_json::from_slice(&msg).unwrap();
        if let A2AEnvelope::SkillAnnouncement(ann) = envelope {
            assert_eq!(ann.skill.skill_id, "test-skill");
            assert!(!ann.is_update);
        } else {
            panic!("expected SkillAnnouncement envelope");
        }
    }

    #[test]
    fn trust_set_returns_clone() {
        let mp = SkillMarketplace::new();
        mp.trust("02aa");
        mp.trust("02bb");
        let set = mp.trust_set();
        assert_eq!(set.len(), 2);
        assert!(set.contains("02aa"));
        assert!(set.contains("02bb"));
    }

    #[test]
    fn track_skill_overwrites_on_same_id() {
        let mp = SkillMarketplace::new();
        let mut s1 = sample_skill("sum");
        s1.version = "1.0.0".into();
        mp.track_skill(s1);

        let mut s2 = sample_skill("sum");
        s2.version = "2.0.0".into();
        mp.track_skill(s2);

        assert_eq!(mp.get_skill("sum").unwrap().version, "2.0.0");
        assert_eq!(mp.known_skills().len(), 1);
    }
}
