//! Discovery, presence, and registry operations for [`WakuA2ANode`](crate::WakuA2ANode).

use anyhow::{Context, Result};
use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard, PresenceAnnouncement};
use logos_messaging_a2a_transport::Transport;
use std::collections::HashMap;

use crate::presence;
use crate::WakuA2ANode;

impl<T: Transport> WakuA2ANode<T> {
    /// Broadcast this agent's card on the discovery topic.
    ///
    /// Discovery uses raw A2AEnvelope (not SDS-wrapped) since it's a
    /// broadcast to unknown peers who may not speak SDS yet.
    pub async fn announce(&self) -> Result<()> {
        let envelope = A2AEnvelope::AgentCard(self.card.clone());
        let payload = serde_json::to_vec(&envelope).context("Failed to serialize AgentCard")?;
        self.channel
            .transport()
            .publish(topics::DISCOVERY, &payload)
            .await
            .context("Failed to announce AgentCard")?;
        tracing::info!(name = %self.card.name, pubkey = %self.pubkey(), "Announced");
        Ok(())
    }

    /// Discover agents by subscribing to the discovery topic and draining messages.
    pub async fn discover(&self) -> Result<Vec<AgentCard>> {
        let mut rx = self
            .channel
            .transport()
            .subscribe(topics::DISCOVERY)
            .await?;

        let mut cards = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Ok(A2AEnvelope::AgentCard(card)) = serde_json::from_slice(&msg) {
                if card.public_key != self.card.public_key {
                    cards.push(card);
                }
            }
        }

        let _ = self
            .channel
            .transport()
            .unsubscribe(topics::DISCOVERY)
            .await;
        Ok(cards)
    }

    /// Register this node's AgentCard in the persistent registry.
    ///
    /// Returns an error if no registry is configured or if the agent
    /// is already registered (use [`update_registry`](Self::update_registry)
    /// to update an existing registration).
    pub async fn register_in_registry(&self) -> Result<()> {
        let registry = self.registry.as_ref().context("no registry configured")?;
        registry
            .register(self.card.clone())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Update this node's AgentCard in the persistent registry.
    pub async fn update_registry(&self) -> Result<()> {
        let registry = self.registry.as_ref().context("no registry configured")?;
        registry
            .update(self.card.clone())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Remove this node from the persistent registry.
    pub async fn deregister_from_registry(&self) -> Result<()> {
        let registry = self.registry.as_ref().context("no registry configured")?;
        registry
            .deregister(&self.card.public_key)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Discover agents from all sources: Waku ephemeral discovery + persistent registry.
    ///
    /// Deduplicates by public key, preferring the registry version when both exist
    /// (since on-chain data is the source of truth).
    pub async fn discover_all(&self) -> Result<Vec<AgentCard>> {
        let mut by_key: HashMap<String, AgentCard> = HashMap::new();

        // Waku ephemeral discovery first
        let waku_cards = self.discover().await?;
        for card in waku_cards {
            by_key.insert(card.public_key.clone(), card);
        }

        // Registry overwrites (source of truth)
        if let Some(ref registry) = self.registry {
            if let Ok(reg_cards) = registry.list().await {
                for card in reg_cards {
                    if card.public_key != self.card.public_key {
                        by_key.insert(card.public_key.clone(), card);
                    }
                }
            }
        }

        Ok(by_key.into_values().collect())
    }

    /// Default presence TTL in seconds (5 minutes).
    const DEFAULT_PRESENCE_TTL: u64 = 300;

    /// Broadcast a presence announcement on the well-known presence topic.
    ///
    /// Other agents subscribed to the presence topic will update their
    /// `PeerMap` with this node's identity and capabilities.
    pub async fn announce_presence(&self) -> Result<()> {
        self.announce_presence_with_ttl(Self::DEFAULT_PRESENCE_TTL)
            .await
    }

    /// Broadcast a presence announcement with a custom TTL.
    pub async fn announce_presence_with_ttl(&self, ttl_secs: u64) -> Result<()> {
        let mut announcement = PresenceAnnouncement {
            agent_id: self.pubkey().to_string(),
            name: self.card.name.clone(),
            capabilities: self.card.capabilities.clone(),
            waku_topic: topics::task_topic(self.pubkey()),
            ttl_secs,
            signature: None,
        };
        announcement
            .sign(&self.signing_key)
            .context("Failed to sign presence announcement")?;
        let envelope = A2AEnvelope::Presence(announcement);
        let payload =
            serde_json::to_vec(&envelope).context("Failed to serialize presence announcement")?;
        self.channel
            .transport()
            .publish(topics::PRESENCE, &payload)
            .await
            .context("Failed to publish presence announcement")?;
        tracing::info!(name = %self.card.name, ttl_secs, "Presence announced");
        Ok(())
    }

    /// Poll the presence topic for new announcements and update the peer map.
    ///
    /// Call this periodically (or before routing a task) to keep the peer
    /// map fresh. Ignores announcements from this node itself.
    pub async fn poll_presence(&self) -> Result<usize> {
        let mut presence_rx = self.presence_rx.lock().await;
        if presence_rx.is_none() {
            *presence_rx = Some(self.channel.transport().subscribe(topics::PRESENCE).await?);
        }
        let rx = presence_rx.as_mut().unwrap();

        let mut count = 0;
        while let Ok(msg) = rx.try_recv() {
            if let Ok(A2AEnvelope::Presence(ann)) = serde_json::from_slice::<A2AEnvelope>(&msg) {
                if ann.agent_id != self.pubkey() {
                    if let Err(e) = ann.verify() {
                        tracing::warn!(
                            name = %ann.name,
                            agent_id = %&ann.agent_id[..8.min(ann.agent_id.len())],
                            error = %e,
                            "Presence rejected (invalid signature)"
                        );
                        continue;
                    }
                    self.peer_map.update(&ann);
                    tracing::info!(
                        name = %ann.name,
                        agent_id = %&ann.agent_id[..8.min(ann.agent_id.len())],
                        capabilities = ?ann.capabilities,
                        "Presence received"
                    );
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Get a reference to the live peer map.
    pub fn peers(&self) -> &presence::PeerMap {
        &self.peer_map
    }

    /// Find peers by capability from the live peer map.
    pub fn find_peers_by_capability(&self, capability: &str) -> Vec<(String, presence::PeerInfo)> {
        self.peer_map.find_by_capability(capability)
    }
}

#[cfg(test)]
mod registry_tests {
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::registry::{AgentRegistry, InMemoryRegistry};
    use logos_messaging_a2a_core::AgentCard;
    use logos_messaging_a2a_transport::memory::InMemoryTransport;
    use std::sync::Arc;

    fn make_node(name: &str) -> WakuA2ANode<InMemoryTransport> {
        let transport = InMemoryTransport::new();
        WakuA2ANode::new(
            name,
            &format!("{} agent", name),
            vec!["test".into()],
            transport,
        )
    }

    #[tokio::test]
    async fn with_registry_builder() {
        let transport = InMemoryTransport::new();
        let registry = Arc::new(InMemoryRegistry::new());
        let node = WakuA2ANode::new("test", "test agent", vec![], transport)
            .with_registry(registry.clone());
        assert!(node.registry.is_some());
    }

    #[tokio::test]
    async fn register_in_registry_succeeds() {
        let node = make_node("echo");
        let registry = Arc::new(InMemoryRegistry::new());
        let node = node.with_registry(registry.clone());

        node.register_in_registry().await.unwrap();
        let card = registry.get(&node.card.public_key).await.unwrap();
        assert_eq!(card.name, "echo");
    }

    #[tokio::test]
    async fn register_without_registry_fails() {
        let node = make_node("echo");
        let result = node.register_in_registry().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no registry"));
    }

    #[tokio::test]
    async fn update_registry_succeeds() {
        let registry = Arc::new(InMemoryRegistry::new());
        let node = make_node("v1");
        let node = node.with_registry(registry.clone());
        node.register_in_registry().await.unwrap();

        // Simulate card update (change name field by re-registering after update)
        let mut updated_card = node.card.clone();
        updated_card.name = "v2".into();
        registry.update(updated_card.clone()).await.unwrap();

        let got = registry.get(&node.card.public_key).await.unwrap();
        assert_eq!(got.name, "v2");
    }

    #[tokio::test]
    async fn deregister_from_registry_succeeds() {
        let registry = Arc::new(InMemoryRegistry::new());
        let node = make_node("temp").with_registry(registry.clone());
        node.register_in_registry().await.unwrap();
        node.deregister_from_registry().await.unwrap();

        let result = registry.get(&node.card.public_key).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn discover_all_merges_waku_and_registry() {
        let transport = InMemoryTransport::new();
        let registry = Arc::new(InMemoryRegistry::new());

        // Register an agent in the registry
        let reg_card = AgentCard {
            name: "registry-agent".into(),
            description: "from registry".into(),
            version: "1.0.0".into(),
            capabilities: vec!["search".into()],
            public_key: "registry_key_001".into(),
            intro_bundle: None,
        };
        registry.register(reg_card).await.unwrap();

        let node =
            WakuA2ANode::new("discoverer", "disc", vec![], transport).with_registry(registry);

        let all = node.discover_all().await.unwrap();
        // Should find the registry agent
        assert!(all.iter().any(|c| c.name == "registry-agent"));
    }

    #[tokio::test]
    async fn discover_all_excludes_self_from_registry() {
        let transport = InMemoryTransport::new();
        let registry = Arc::new(InMemoryRegistry::new());

        let node =
            WakuA2ANode::new("self-node", "me", vec![], transport).with_registry(registry.clone());

        // Register self in registry
        node.register_in_registry().await.unwrap();

        let all = node.discover_all().await.unwrap();
        // Should NOT find self
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn discover_all_without_registry_returns_waku_only() {
        let node = make_node("plain");
        // No registry set — should still work, just return Waku results
        let all = node.discover_all().await.unwrap();
        assert!(all.is_empty()); // no Waku broadcasts either
    }
}

#[cfg(test)]
mod signed_presence_tests {
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::{topics, A2AEnvelope, PresenceAnnouncement};
    use logos_messaging_a2a_transport::memory::InMemoryTransport;
    use logos_messaging_a2a_transport::Transport;

    fn make_node_with_transport(
        name: &str,
        transport: InMemoryTransport,
    ) -> WakuA2ANode<InMemoryTransport> {
        WakuA2ANode::new(
            name,
            &format!("{name} agent"),
            vec!["test".into()],
            transport,
        )
    }

    #[tokio::test]
    async fn signed_announcement_accepted_by_peer() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        // Alice announces (signed automatically)
        alice.announce_presence().await.unwrap();

        // Bob polls — should accept the signed announcement
        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 1);

        let peers = bob.peers().all_live();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].1.name, "alice");
    }

    #[tokio::test]
    async fn unsigned_announcement_rejected() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        // Inject an unsigned announcement directly (bypassing sign)
        let unsigned = PresenceAnnouncement {
            agent_id: alice.pubkey().to_string(),
            name: "alice".to_string(),
            capabilities: vec!["test".into()],
            waku_topic: topics::task_topic(alice.pubkey()),
            ttl_secs: 300,
            signature: None,
        };
        let envelope = A2AEnvelope::Presence(unsigned);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(topics::PRESENCE, &payload).await.unwrap();

        // Bob should reject the unsigned announcement
        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 0);
        assert!(bob.peers().all_live().is_empty());
    }

    #[tokio::test]
    async fn tampered_announcement_rejected() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        // Create a properly signed announcement, then tamper with it
        let mut ann = PresenceAnnouncement {
            agent_id: alice.pubkey().to_string(),
            name: "alice".to_string(),
            capabilities: vec!["test".into()],
            waku_topic: topics::task_topic(alice.pubkey()),
            ttl_secs: 300,
            signature: None,
        };
        ann.sign(alice.signing_key()).unwrap();

        // Tamper with the name after signing
        ann.name = "evil-alice".to_string();

        let envelope = A2AEnvelope::Presence(ann);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(topics::PRESENCE, &payload).await.unwrap();

        // Bob should reject the tampered announcement
        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 0);
        assert!(bob.peers().all_live().is_empty());
    }

    #[tokio::test]
    async fn wrong_key_announcement_rejected() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        // Sign with bob's key but claim to be alice
        let mut ann = PresenceAnnouncement {
            agent_id: alice.pubkey().to_string(),
            name: "alice".to_string(),
            capabilities: vec!["test".into()],
            waku_topic: topics::task_topic(alice.pubkey()),
            ttl_secs: 300,
            signature: None,
        };
        ann.sign(bob.signing_key()).unwrap();

        let envelope = A2AEnvelope::Presence(ann);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(topics::PRESENCE, &payload).await.unwrap();

        // Bob should reject — signature doesn't match agent_id
        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 0);
        assert!(bob.peers().all_live().is_empty());
    }
}
