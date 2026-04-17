//! Skill registry trait for on-chain skill registration and rating.
//!
//! The [`SkillRegistry`] trait defines a persistent store for skill
//! descriptors and ratings. The primary implementation target is a LEZ
//! program, but any persistent store can implement this trait.

use crate::skill::{SkillDescriptor, SkillRating};
use std::fmt;

/// Errors returned by skill registry operations.
#[derive(Debug)]
pub enum SkillRegistryError {
    /// Skill with this ID is not registered.
    NotFound(String),
    /// Only the original author can update a skill.
    Unauthorized(String),
    /// Skill with this ID and version is already registered.
    AlreadyRegistered(String),
    /// Rating validation failed (e.g. score out of range).
    InvalidRating(String),
    /// Network or backend failure.
    Backend(String),
}

impl fmt::Display for SkillRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "skill not found: {}", id),
            Self::Unauthorized(msg) => write!(f, "unauthorized: {}", msg),
            Self::AlreadyRegistered(id) => write!(f, "skill already registered: {}", id),
            Self::InvalidRating(msg) => write!(f, "invalid rating: {}", msg),
            Self::Backend(msg) => write!(f, "skill registry backend error: {}", msg),
        }
    }
}

impl std::error::Error for SkillRegistryError {}

/// Persistent skill registry for on-chain or off-chain skill management.
#[async_trait::async_trait]
pub trait SkillRegistry: Send + Sync {
    /// Register a new skill. Returns error if already registered.
    async fn register(&self, skill: SkillDescriptor) -> Result<(), SkillRegistryError>;

    /// Update an existing skill (new version). Only the original author may update.
    async fn update(&self, skill: SkillDescriptor) -> Result<(), SkillRegistryError>;

    /// Remove a skill from the registry.
    async fn deregister(&self, skill_id: &str) -> Result<(), SkillRegistryError>;

    /// Look up a skill by ID.
    async fn get(&self, skill_id: &str) -> Result<SkillDescriptor, SkillRegistryError>;

    /// List all registered skills.
    async fn list(&self) -> Result<Vec<SkillDescriptor>, SkillRegistryError>;

    /// Search skills by tag.
    async fn find_by_tag(&self, tag: &str) -> Result<Vec<SkillDescriptor>, SkillRegistryError>;

    /// Submit a rating for a skill. One rating per (skill_id, rater_pubkey) —
    /// latest wins.
    async fn rate(&self, rating: SkillRating) -> Result<(), SkillRegistryError>;

    /// Get all ratings for a skill.
    async fn get_ratings(
        &self,
        skill_id: &str,
    ) -> Result<Vec<SkillRating>, SkillRegistryError>;
}

/// In-memory skill registry for testing and local development.
pub struct InMemorySkillRegistry {
    skills: std::sync::RwLock<std::collections::HashMap<String, SkillDescriptor>>,
    /// Ratings keyed by (skill_id, rater_pubkey).
    ratings: std::sync::RwLock<std::collections::HashMap<(String, String), SkillRating>>,
}

impl InMemorySkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: std::sync::RwLock::new(std::collections::HashMap::new()),
            ratings: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for InMemorySkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SkillRegistry for InMemorySkillRegistry {
    async fn register(&self, skill: SkillDescriptor) -> Result<(), SkillRegistryError> {
        let mut skills = self.skills.write().unwrap();
        if skills.contains_key(&skill.skill_id) {
            return Err(SkillRegistryError::AlreadyRegistered(skill.skill_id));
        }
        skills.insert(skill.skill_id.clone(), skill);
        Ok(())
    }

    async fn update(&self, skill: SkillDescriptor) -> Result<(), SkillRegistryError> {
        let mut skills = self.skills.write().unwrap();
        match skills.get(&skill.skill_id) {
            None => return Err(SkillRegistryError::NotFound(skill.skill_id)),
            Some(existing) if existing.author_pubkey != skill.author_pubkey => {
                return Err(SkillRegistryError::Unauthorized(
                    "only the original author can update".into(),
                ));
            }
            _ => {}
        }
        skills.insert(skill.skill_id.clone(), skill);
        Ok(())
    }

    async fn deregister(&self, skill_id: &str) -> Result<(), SkillRegistryError> {
        let mut skills = self.skills.write().unwrap();
        skills
            .remove(skill_id)
            .ok_or_else(|| SkillRegistryError::NotFound(skill_id.to_string()))?;
        Ok(())
    }

    async fn get(&self, skill_id: &str) -> Result<SkillDescriptor, SkillRegistryError> {
        let skills = self.skills.read().unwrap();
        skills
            .get(skill_id)
            .cloned()
            .ok_or_else(|| SkillRegistryError::NotFound(skill_id.to_string()))
    }

    async fn list(&self) -> Result<Vec<SkillDescriptor>, SkillRegistryError> {
        let skills = self.skills.read().unwrap();
        Ok(skills.values().cloned().collect())
    }

    async fn find_by_tag(&self, tag: &str) -> Result<Vec<SkillDescriptor>, SkillRegistryError> {
        let skills = self.skills.read().unwrap();
        Ok(skills
            .values()
            .filter(|s| s.tags.iter().any(|t| t == tag))
            .cloned()
            .collect())
    }

    async fn rate(&self, rating: SkillRating) -> Result<(), SkillRegistryError> {
        if !rating.is_valid() {
            return Err(SkillRegistryError::InvalidRating(format!(
                "score {} out of range 1..=5",
                rating.score
            )));
        }
        // Skill must exist.
        {
            let skills = self.skills.read().unwrap();
            if !skills.contains_key(&rating.skill_id) {
                return Err(SkillRegistryError::NotFound(rating.skill_id.clone()));
            }
        }
        let key = (rating.skill_id.clone(), rating.rater_pubkey.clone());
        let mut ratings = self.ratings.write().unwrap();
        ratings.insert(key, rating);
        Ok(())
    }

    async fn get_ratings(
        &self,
        skill_id: &str,
    ) -> Result<Vec<SkillRating>, SkillRegistryError> {
        let ratings = self.ratings.read().unwrap();
        Ok(ratings
            .values()
            .filter(|r| r.skill_id == skill_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_skill(id: &str, author: &str) -> SkillDescriptor {
        SkillDescriptor {
            skill_id: id.into(),
            content_hash: "bafytest".into(),
            author_pubkey: author.into(),
            version: "1.0.0".into(),
            description: format!("{} skill", id),
            tags: vec!["test".into()],
            timestamp: 1000,
        }
    }

    #[tokio::test]
    async fn register_and_get() {
        let reg = InMemorySkillRegistry::new();
        let skill = sample_skill("sum", "02aa");
        reg.register(skill.clone()).await.unwrap();
        let got = reg.get("sum").await.unwrap();
        assert_eq!(got, skill);
    }

    #[tokio::test]
    async fn register_duplicate_fails() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("sum", "02aa")).await.unwrap();
        let err = reg.register(sample_skill("sum", "02aa")).await.unwrap_err();
        assert!(matches!(err, SkillRegistryError::AlreadyRegistered(_)));
    }

    #[tokio::test]
    async fn update_by_author_succeeds() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("sum", "02aa")).await.unwrap();
        let mut updated = sample_skill("sum", "02aa");
        updated.version = "2.0.0".into();
        reg.update(updated).await.unwrap();
        let got = reg.get("sum").await.unwrap();
        assert_eq!(got.version, "2.0.0");
    }

    #[tokio::test]
    async fn update_by_different_author_fails() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("sum", "02aa")).await.unwrap();
        let err = reg
            .update(sample_skill("sum", "02bb"))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillRegistryError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn update_nonexistent_fails() {
        let reg = InMemorySkillRegistry::new();
        let err = reg
            .update(sample_skill("ghost", "02aa"))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillRegistryError::NotFound(_)));
    }

    #[tokio::test]
    async fn deregister_removes() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("sum", "02aa")).await.unwrap();
        reg.deregister("sum").await.unwrap();
        assert!(reg.get("sum").await.is_err());
    }

    #[tokio::test]
    async fn find_by_tag() {
        let reg = InMemorySkillRegistry::new();
        let mut s1 = sample_skill("a", "02aa");
        s1.tags = vec!["nlp".into()];
        let mut s2 = sample_skill("b", "02bb");
        s2.tags = vec!["code".into()];
        let mut s3 = sample_skill("c", "02cc");
        s3.tags = vec!["nlp".into(), "code".into()];
        reg.register(s1).await.unwrap();
        reg.register(s2).await.unwrap();
        reg.register(s3).await.unwrap();

        let nlp = reg.find_by_tag("nlp").await.unwrap();
        assert_eq!(nlp.len(), 2);
        let code = reg.find_by_tag("code").await.unwrap();
        assert_eq!(code.len(), 2);
        let none = reg.find_by_tag("missing").await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn rate_and_get_ratings() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("sum", "02aa")).await.unwrap();

        let r1 = SkillRating {
            skill_id: "sum".into(),
            rater_pubkey: "02bb".into(),
            score: 5,
            comment: Some("great".into()),
            timestamp: 2000,
        };
        let r2 = SkillRating {
            skill_id: "sum".into(),
            rater_pubkey: "02cc".into(),
            score: 3,
            comment: None,
            timestamp: 2001,
        };
        reg.rate(r1).await.unwrap();
        reg.rate(r2).await.unwrap();

        let ratings = reg.get_ratings("sum").await.unwrap();
        assert_eq!(ratings.len(), 2);
    }

    #[tokio::test]
    async fn rate_replaces_previous() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("sum", "02aa")).await.unwrap();

        let r1 = SkillRating {
            skill_id: "sum".into(),
            rater_pubkey: "02bb".into(),
            score: 2,
            comment: None,
            timestamp: 1000,
        };
        let r2 = SkillRating {
            skill_id: "sum".into(),
            rater_pubkey: "02bb".into(),
            score: 5,
            comment: None,
            timestamp: 2000,
        };
        reg.rate(r1).await.unwrap();
        reg.rate(r2).await.unwrap();

        let ratings = reg.get_ratings("sum").await.unwrap();
        assert_eq!(ratings.len(), 1);
        assert_eq!(ratings[0].score, 5);
    }

    #[tokio::test]
    async fn rate_invalid_score_fails() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("sum", "02aa")).await.unwrap();

        let bad = SkillRating {
            skill_id: "sum".into(),
            rater_pubkey: "02bb".into(),
            score: 0,
            comment: None,
            timestamp: 1000,
        };
        let err = reg.rate(bad).await.unwrap_err();
        assert!(matches!(err, SkillRegistryError::InvalidRating(_)));
    }

    #[tokio::test]
    async fn rate_nonexistent_skill_fails() {
        let reg = InMemorySkillRegistry::new();
        let bad = SkillRating {
            skill_id: "ghost".into(),
            rater_pubkey: "02bb".into(),
            score: 3,
            comment: None,
            timestamp: 1000,
        };
        let err = reg.rate(bad).await.unwrap_err();
        assert!(matches!(err, SkillRegistryError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_skills() {
        let reg = InMemorySkillRegistry::new();
        reg.register(sample_skill("a", "02aa")).await.unwrap();
        reg.register(sample_skill("b", "02bb")).await.unwrap();
        let all = reg.list().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn error_display() {
        assert_eq!(
            SkillRegistryError::NotFound("x".into()).to_string(),
            "skill not found: x"
        );
        assert_eq!(
            SkillRegistryError::Unauthorized("u".into()).to_string(),
            "unauthorized: u"
        );
        assert_eq!(
            SkillRegistryError::AlreadyRegistered("a".into()).to_string(),
            "skill already registered: a"
        );
        assert_eq!(
            SkillRegistryError::InvalidRating("bad".into()).to_string(),
            "invalid rating: bad"
        );
        assert_eq!(
            SkillRegistryError::Backend("err".into()).to_string(),
            "skill registry backend error: err"
        );
    }

    #[test]
    fn default_creates_empty() {
        let reg = InMemorySkillRegistry::default();
        let skills = reg.skills.read().unwrap();
        assert!(skills.is_empty());
    }
}
