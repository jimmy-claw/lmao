//! Agent registry trait for persistent on-chain discovery.
//!
//! While Waku presence broadcasts provide ephemeral, real-time agent
//! discovery, the [`AgentRegistry`] trait defines a persistent store
//! where agents register themselves for long-lived discoverability.
//!
//! The primary implementation target is the LEZ on-chain program registry
//! (SPELbook), where `AgentCard`s are stored as PDA accounts. This module
//! provides the trait and a test-friendly in-memory implementation.
//!
//! # Architecture
//!
//! ```text
//! Discovery layer:
//!   Waku presence (ephemeral) ──┐
//!                                ├──► merged agent list
//!   LEZ registry (persistent) ──┘
//! ```
//!
//! # Example
//!
//! ```
//! use logos_messaging_a2a_core::registry::InMemoryRegistry;
//! use logos_messaging_a2a_core::registry::AgentRegistry;
//! use logos_messaging_a2a_core::AgentCard;
//!
//! # async fn example() -> Result<(), logos_messaging_a2a_core::registry::RegistryError> {
//! let registry = InMemoryRegistry::new();
//! let card = AgentCard {
//!     name: "echo-agent".into(),
//!     description: "Echoes messages".into(),
//!     version: "0.1.0".into(),
//!     capabilities: vec!["echo".into()],
//!     public_key: "deadbeef".into(),
//!     intro_bundle: None,
//! };
//! registry.register(card.clone()).await?;
//! let agents = registry.list().await?;
//! assert_eq!(agents.len(), 1);
//! # Ok(())
//! # }
//! ```

use crate::AgentCard;
use std::fmt;

/// Errors returned by registry operations.
#[derive(Debug)]
pub enum RegistryError {
    /// Agent with this public key is not registered.
    NotFound(String),
    /// Only the original registrant can update/deregister.
    Unauthorized(String),
    /// Agent with this public key is already registered.
    AlreadyRegistered(String),
    /// Network or backend failure.
    Backend(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::NotFound(key) => write!(f, "agent not found: {}", key),
            RegistryError::Unauthorized(msg) => write!(f, "unauthorized: {}", msg),
            RegistryError::AlreadyRegistered(key) => {
                write!(f, "agent already registered: {}", key)
            }
            RegistryError::Backend(msg) => write!(f, "registry backend error: {}", msg),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Persistent agent registry for on-chain or off-chain discovery.
///
/// Implementors provide durable storage of [`AgentCard`]s. The primary
/// target is LEZ (via SPELbook PDA accounts), but any persistent store
/// (database, file, etc.) can implement this trait.
#[async_trait::async_trait]
pub trait AgentRegistry: Send + Sync {
    /// Register an agent. Returns error if already registered.
    async fn register(&self, card: AgentCard) -> Result<(), RegistryError>;

    /// Update an existing registration. Returns error if not found.
    async fn update(&self, card: AgentCard) -> Result<(), RegistryError>;

    /// Remove an agent from the registry.
    async fn deregister(&self, public_key: &str) -> Result<(), RegistryError>;

    /// Look up a specific agent by public key.
    async fn get(&self, public_key: &str) -> Result<AgentCard, RegistryError>;

    /// List all registered agents.
    async fn list(&self) -> Result<Vec<AgentCard>, RegistryError>;

    /// Search agents by capability tag.
    async fn find_by_capability(&self, capability: &str) -> Result<Vec<AgentCard>, RegistryError>;
}

/// In-memory agent registry for testing and local development.
///
/// Not persistent — data is lost when the process exits.
pub struct InMemoryRegistry {
    agents: std::sync::RwLock<std::collections::HashMap<String, AgentCard>>,
}

impl InMemoryRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            agents: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for InMemoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl AgentRegistry for InMemoryRegistry {
    async fn register(&self, card: AgentCard) -> Result<(), RegistryError> {
        let mut agents = self.agents.write().unwrap();
        if agents.contains_key(&card.public_key) {
            return Err(RegistryError::AlreadyRegistered(card.public_key));
        }
        agents.insert(card.public_key.clone(), card);
        Ok(())
    }

    async fn update(&self, card: AgentCard) -> Result<(), RegistryError> {
        let mut agents = self.agents.write().unwrap();
        if !agents.contains_key(&card.public_key) {
            return Err(RegistryError::NotFound(card.public_key));
        }
        agents.insert(card.public_key.clone(), card);
        Ok(())
    }

    async fn deregister(&self, public_key: &str) -> Result<(), RegistryError> {
        let mut agents = self.agents.write().unwrap();
        agents
            .remove(public_key)
            .ok_or_else(|| RegistryError::NotFound(public_key.to_string()))?;
        Ok(())
    }

    async fn get(&self, public_key: &str) -> Result<AgentCard, RegistryError> {
        let agents = self.agents.read().unwrap();
        agents
            .get(public_key)
            .cloned()
            .ok_or_else(|| RegistryError::NotFound(public_key.to_string()))
    }

    async fn list(&self) -> Result<Vec<AgentCard>, RegistryError> {
        let agents = self.agents.read().unwrap();
        Ok(agents.values().cloned().collect())
    }

    async fn find_by_capability(&self, capability: &str) -> Result<Vec<AgentCard>, RegistryError> {
        let agents = self.agents.read().unwrap();
        Ok(agents
            .values()
            .filter(|c| c.capabilities.iter().any(|cap| cap == capability))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_card(name: &str, pubkey: &str, caps: Vec<&str>) -> AgentCard {
        AgentCard {
            name: name.to_string(),
            description: format!("{} agent", name),
            version: "0.1.0".to_string(),
            capabilities: caps.into_iter().map(String::from).collect(),
            public_key: pubkey.to_string(),
            intro_bundle: None,
        }
    }

    #[tokio::test]
    async fn register_and_get() {
        let reg = InMemoryRegistry::new();
        let card = test_card("echo", "aabb", vec!["echo"]);
        reg.register(card.clone()).await.unwrap();
        let got = reg.get("aabb").await.unwrap();
        assert_eq!(got, card);
    }

    #[tokio::test]
    async fn register_duplicate_fails() {
        let reg = InMemoryRegistry::new();
        let card = test_card("echo", "aabb", vec!["echo"]);
        reg.register(card.clone()).await.unwrap();
        let err = reg.register(card).await.unwrap_err();
        assert!(matches!(err, RegistryError::AlreadyRegistered(_)));
    }

    #[tokio::test]
    async fn update_existing() {
        let reg = InMemoryRegistry::new();
        let card = test_card("echo", "aabb", vec!["echo"]);
        reg.register(card).await.unwrap();

        let updated = test_card("echo-v2", "aabb", vec!["echo", "translate"]);
        reg.update(updated.clone()).await.unwrap();

        let got = reg.get("aabb").await.unwrap();
        assert_eq!(got.name, "echo-v2");
        assert_eq!(got.capabilities.len(), 2);
    }

    #[tokio::test]
    async fn update_nonexistent_fails() {
        let reg = InMemoryRegistry::new();
        let card = test_card("ghost", "ccdd", vec!["spook"]);
        let err = reg.update(card).await.unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
    }

    #[tokio::test]
    async fn deregister_removes() {
        let reg = InMemoryRegistry::new();
        let card = test_card("echo", "aabb", vec!["echo"]);
        reg.register(card).await.unwrap();
        reg.deregister("aabb").await.unwrap();
        let err = reg.get("aabb").await.unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
    }

    #[tokio::test]
    async fn deregister_nonexistent_fails() {
        let reg = InMemoryRegistry::new();
        let err = reg.deregister("nope").await.unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_all_agents() {
        let reg = InMemoryRegistry::new();
        reg.register(test_card("a", "11", vec!["x"])).await.unwrap();
        reg.register(test_card("b", "22", vec!["y"])).await.unwrap();
        reg.register(test_card("c", "33", vec!["x", "y"]))
            .await
            .unwrap();
        let all = reg.list().await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn find_by_capability_filters() {
        let reg = InMemoryRegistry::new();
        reg.register(test_card("a", "11", vec!["translate"]))
            .await
            .unwrap();
        reg.register(test_card("b", "22", vec!["echo"]))
            .await
            .unwrap();
        reg.register(test_card("c", "33", vec!["translate", "echo"]))
            .await
            .unwrap();

        let translators = reg.find_by_capability("translate").await.unwrap();
        assert_eq!(translators.len(), 2);

        let echoers = reg.find_by_capability("echo").await.unwrap();
        assert_eq!(echoers.len(), 2);

        let none = reg.find_by_capability("nonexistent").await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn get_nonexistent_fails() {
        let reg = InMemoryRegistry::new();
        let err = reg.get("missing").await.unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
    }

    #[test]
    fn registry_error_display() {
        let e1 = RegistryError::NotFound("abc".into());
        assert!(e1.to_string().contains("abc"));
        let e2 = RegistryError::Unauthorized("no".into());
        assert!(e2.to_string().contains("no"));
        let e3 = RegistryError::AlreadyRegistered("xyz".into());
        assert!(e3.to_string().contains("xyz"));
        let e4 = RegistryError::Backend("timeout".into());
        assert!(e4.to_string().contains("timeout"));
    }

    #[test]
    fn default_creates_empty_registry() {
        let reg = InMemoryRegistry::default();
        let agents = reg.agents.read().unwrap();
        assert!(agents.is_empty());
    }
}
