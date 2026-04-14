//! Ephemeral multi-agent group sessions.
//!
//! Foundation for MLS group chat support (Chat SDK v0.2). A [`GroupSession`]
//! tracks the members of a task-scoped agent group, where membership can change
//! dynamically as agents join or leave.
//!
//! Currently uses pairwise ChatSession keys for each member pair. When Chat SDK
//! ships MLS (v0.2), this will be upgraded to true group key agreement.

use logos_messaging_a2a_crypto::IntroBundle;
use serde::{Deserialize, Serialize};

/// Errors from [`GroupSession`] operations.
#[derive(Debug, thiserror::Error)]
pub enum GroupSessionError {
    #[error("member already in group: {0}")]
    AlreadyMember(String),
    #[error("member not in group: {0}")]
    NotMember(String),
    #[error("group is empty")]
    EmptyGroup,
}

/// A member of a group session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupMember {
    /// The member's X25519 public key (hex).
    pub pubkey: String,
    /// Human-readable agent name.
    pub name: String,
    /// The member's intro bundle for pairwise session establishment.
    pub intro_bundle: IntroBundle,
}

/// An ephemeral multi-agent group session.
///
/// Tracks members participating in a task-scoped group. Agents join with their
/// intro bundle and can be removed when they leave.
///
/// # Future: MLS upgrade
///
/// When Chat SDK v0.2 ships MLS group key agreement, `GroupSession` will be
/// upgraded from pairwise keys to a single group key. The API surface will
/// remain the same — `add_member` / `remove_member` / `members`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSession {
    /// Unique group session identifier.
    pub id: String,
    /// Human-readable group name / purpose.
    pub name: String,
    /// Current group members.
    members: Vec<GroupMember>,
}

impl GroupSession {
    /// Create a new empty group session.
    pub fn new(name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            members: Vec::new(),
        }
    }

    /// Add a member to the group.
    pub fn add_member(&mut self, member: GroupMember) -> Result<(), GroupSessionError> {
        if self.members.iter().any(|m| m.pubkey == member.pubkey) {
            return Err(GroupSessionError::AlreadyMember(member.pubkey));
        }
        self.members.push(member);
        Ok(())
    }

    /// Remove a member by public key.
    pub fn remove_member(&mut self, pubkey: &str) -> Result<GroupMember, GroupSessionError> {
        let idx = self
            .members
            .iter()
            .position(|m| m.pubkey == pubkey)
            .ok_or_else(|| GroupSessionError::NotMember(pubkey.to_string()))?;
        Ok(self.members.remove(idx))
    }

    /// Current group members.
    pub fn members(&self) -> &[GroupMember] {
        &self.members
    }

    /// Number of members in the group.
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Whether a public key is a member of the group.
    pub fn is_member(&self, pubkey: &str) -> bool {
        self.members.iter().any(|m| m.pubkey == pubkey)
    }

    /// Get all intro bundles for establishing pairwise sessions with group members.
    pub fn intro_bundles(&self) -> Vec<&IntroBundle> {
        self.members.iter().map(|m| &m.intro_bundle).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_member(name: &str, pubkey: &str) -> GroupMember {
        GroupMember {
            pubkey: pubkey.to_string(),
            name: name.to_string(),
            intro_bundle: IntroBundle::new(pubkey),
        }
    }

    #[test]
    fn new_group_is_empty() {
        let group = GroupSession::new("task-123");
        assert_eq!(group.member_count(), 0);
        assert!(group.members().is_empty());
    }

    #[test]
    fn add_and_remove_members() {
        let mut group = GroupSession::new("task");
        let alice = test_member("alice", "aa");
        let bob = test_member("bob", "bb");

        group.add_member(alice).unwrap();
        group.add_member(bob).unwrap();
        assert_eq!(group.member_count(), 2);
        assert!(group.is_member("aa"));
        assert!(group.is_member("bb"));

        let removed = group.remove_member("aa").unwrap();
        assert_eq!(removed.name, "alice");
        assert_eq!(group.member_count(), 1);
        assert!(!group.is_member("aa"));
    }

    #[test]
    fn duplicate_member_rejected() {
        let mut group = GroupSession::new("task");
        let alice = test_member("alice", "aa");
        group.add_member(alice.clone()).unwrap();

        let result = group.add_member(alice);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already"));
    }

    #[test]
    fn remove_nonexistent_member_fails() {
        let mut group = GroupSession::new("task");
        let result = group.remove_member("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in group"));
    }

    #[test]
    fn intro_bundles_returns_all_member_bundles() {
        let mut group = GroupSession::new("task");
        group.add_member(test_member("a", "aa")).unwrap();
        group.add_member(test_member("b", "bb")).unwrap();
        group.add_member(test_member("c", "cc")).unwrap();

        let bundles = group.intro_bundles();
        assert_eq!(bundles.len(), 3);
        let pubkeys: Vec<&str> = bundles.iter().map(|b| b.agent_pubkey.as_str()).collect();
        assert!(pubkeys.contains(&"aa"));
        assert!(pubkeys.contains(&"bb"));
        assert!(pubkeys.contains(&"cc"));
    }

    #[test]
    fn group_session_serialization() {
        let mut group = GroupSession::new("task-42");
        group.add_member(test_member("alice", "aa")).unwrap();
        group.add_member(test_member("bob", "bb")).unwrap();

        let json = serde_json::to_string(&group).unwrap();
        let deserialized: GroupSession = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "task-42");
        assert_eq!(deserialized.members.len(), 2);
    }

    #[test]
    fn group_id_is_uuid() {
        let group = GroupSession::new("test");
        assert_eq!(group.id.len(), 36);
        assert_eq!(group.id.chars().filter(|c| *c == '-').count(), 4);
    }

    #[test]
    fn dynamic_membership_changes() {
        let mut group = GroupSession::new("ephemeral-task");

        // Agents join as task starts
        group.add_member(test_member("coordinator", "c1")).unwrap();
        group.add_member(test_member("worker-1", "w1")).unwrap();
        group.add_member(test_member("worker-2", "w2")).unwrap();
        assert_eq!(group.member_count(), 3);

        // Worker finishes and leaves
        group.remove_member("w1").unwrap();
        assert_eq!(group.member_count(), 2);

        // New worker joins
        group.add_member(test_member("worker-3", "w3")).unwrap();
        assert_eq!(group.member_count(), 3);
        assert!(!group.is_member("w1"));
        assert!(group.is_member("w3"));
    }
}
