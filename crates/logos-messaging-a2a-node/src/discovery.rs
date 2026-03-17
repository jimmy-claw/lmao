//! Discovery, presence, and registry operations for [`WakuA2ANode`].

use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard, PresenceAnnouncement};
use logos_messaging_a2a_transport::Transport;
use std::collections::HashMap;

use crate::metrics::Metrics;
use crate::presence;
use crate::{NodeError, Result, WakuA2ANode};

impl<T: Transport> WakuA2ANode<T> {
    /// Broadcast this agent's card on the discovery topic.
    ///
    /// Discovery uses raw A2AEnvelope (not SDS-wrapped) since it's a
    /// broadcast to unknown peers who may not speak SDS yet.
    pub async fn announce(&self) -> Result<()> {
        let envelope = A2AEnvelope::AgentCard(self.card.clone());
        let payload = serde_json::to_vec(&envelope)?;
        self.channel
            .transport()
            .publish(topics::DISCOVERY, &payload)
            .await?;
        Metrics::inc(&self.metrics.announcements_sent);
        Metrics::inc(&self.metrics.messages_published);
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
        Metrics::inc_by(&self.metrics.discoveries, cards.len() as u64);
        Ok(cards)
    }

    /// Register this node's AgentCard in the persistent registry.
    ///
    /// Returns an error if no registry is configured or if the agent
    /// is already registered (use [`update_registry`](Self::update_registry)
    /// to update an existing registration).
    pub async fn register_in_registry(&self) -> Result<()> {
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| NodeError::Other("no registry configured".into()))?;
        registry
            .register(self.card.clone())
            .await
            .map_err(|e| NodeError::Other(format!("{}", e)))
    }

    /// Update this node's AgentCard in the persistent registry.
    pub async fn update_registry(&self) -> Result<()> {
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| NodeError::Other("no registry configured".into()))?;
        registry
            .update(self.card.clone())
            .await
            .map_err(|e| NodeError::Other(format!("{}", e)))
    }

    /// Remove this node from the persistent registry.
    pub async fn deregister_from_registry(&self) -> Result<()> {
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| NodeError::Other("no registry configured".into()))?;
        registry
            .deregister(&self.card.public_key)
            .await
            .map_err(|e| NodeError::Other(format!("{}", e)))
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
        announcement.sign(&self.signing_key)?;
        let envelope = A2AEnvelope::Presence(announcement);
        let payload = serde_json::to_vec(&envelope)?;
        self.channel
            .transport()
            .publish(topics::PRESENCE, &payload)
            .await?;
        Metrics::inc(&self.metrics.announcements_sent);
        Metrics::inc(&self.metrics.messages_published);
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
                    Metrics::inc(&self.metrics.peers_discovered);
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

#[cfg(test)]
mod announce_and_discover_tests {
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard};
    use logos_messaging_a2a_transport::memory::InMemoryTransport;
    use logos_messaging_a2a_transport::Transport;

    fn make_node(name: &str) -> WakuA2ANode<InMemoryTransport> {
        let transport = InMemoryTransport::new();
        WakuA2ANode::new(
            name,
            &format!("{name} agent"),
            vec!["test".into()],
            transport,
        )
    }

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

    // --- announce() tests ---

    #[tokio::test]
    async fn announce_publishes_agent_card_to_discovery_topic() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("echo", transport.clone());

        node.announce().await.unwrap();

        let mut rx = transport.subscribe(topics::DISCOVERY).await.unwrap();
        let msg = rx.try_recv().unwrap();
        let envelope: A2AEnvelope = serde_json::from_slice(&msg).unwrap();
        match envelope {
            A2AEnvelope::AgentCard(card) => {
                assert_eq!(card.name, "echo");
                assert_eq!(card.public_key, node.card.public_key);
                assert_eq!(card.capabilities, vec!["test"]);
            }
            other => panic!("expected AgentCard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn announce_increments_metrics() {
        let node = make_node("echo");

        let before = node.metrics();
        assert_eq!(before.announcements_sent, 0);
        assert_eq!(before.messages_published, 0);

        node.announce().await.unwrap();

        let after = node.metrics();
        assert_eq!(after.announcements_sent, 1);
        assert_eq!(after.messages_published, 1);
    }

    #[tokio::test]
    async fn announce_multiple_times_publishes_multiple_messages() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("echo", transport.clone());

        node.announce().await.unwrap();
        node.announce().await.unwrap();
        node.announce().await.unwrap();

        let mut rx = transport.subscribe(topics::DISCOVERY).await.unwrap();
        let mut count = 0;
        while rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
        assert_eq!(node.metrics().announcements_sent, 3);
    }

    // --- discover() tests ---

    #[tokio::test]
    async fn discover_returns_other_agents_cards() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        alice.announce().await.unwrap();

        let cards = bob.discover().await.unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "alice");
        assert_eq!(cards[0].public_key, alice.card.public_key);
    }

    #[tokio::test]
    async fn discover_excludes_self() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("self-announcer", transport.clone());

        node.announce().await.unwrap();

        let cards = node.discover().await.unwrap();
        assert!(cards.is_empty());
    }

    #[tokio::test]
    async fn discover_multiple_agents() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());
        let carol = make_node_with_transport("carol", transport.clone());
        let discoverer = make_node_with_transport("discoverer", transport.clone());

        alice.announce().await.unwrap();
        bob.announce().await.unwrap();
        carol.announce().await.unwrap();

        let cards = discoverer.discover().await.unwrap();
        assert_eq!(cards.len(), 3);

        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"alice"));
        assert!(names.contains(&"bob"));
        assert!(names.contains(&"carol"));
    }

    #[tokio::test]
    async fn discover_empty_when_no_messages() {
        let node = make_node("lonely");
        let cards = node.discover().await.unwrap();
        assert!(cards.is_empty());
    }

    #[tokio::test]
    async fn discover_increments_metrics() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());
        let discoverer = make_node_with_transport("discoverer", transport.clone());

        alice.announce().await.unwrap();
        bob.announce().await.unwrap();

        discoverer.discover().await.unwrap();
        assert_eq!(discoverer.metrics().discoveries, 2);
    }

    #[tokio::test]
    async fn discover_ignores_non_agent_card_envelopes() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("discoverer", transport.clone());

        // Inject a non-AgentCard envelope onto the discovery topic
        let task = logos_messaging_a2a_core::Task::new("from", "to", "hello");
        let envelope = A2AEnvelope::Task(task);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport
            .publish(topics::DISCOVERY, &payload)
            .await
            .unwrap();

        // Also inject garbage bytes
        transport
            .publish(topics::DISCOVERY, b"not valid json")
            .await
            .unwrap();

        let cards = node.discover().await.unwrap();
        assert!(cards.is_empty());
    }

    #[tokio::test]
    async fn discover_zero_discoveries_metric_when_empty() {
        let node = make_node("lonely");
        node.discover().await.unwrap();
        assert_eq!(node.metrics().discoveries, 0);
    }

    // --- announce_presence / announce_presence_with_ttl tests ---

    #[tokio::test]
    async fn announce_presence_with_default_ttl() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        alice.announce_presence().await.unwrap();

        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 1);
        let peers = bob.peers().all_live();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].1.ttl_secs, 300);
    }

    #[tokio::test]
    async fn announce_presence_with_custom_ttl() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        alice.announce_presence_with_ttl(60).await.unwrap();

        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 1);
        let peers = bob.peers().all_live();
        assert_eq!(peers[0].1.ttl_secs, 60);
    }

    #[tokio::test]
    async fn announce_presence_increments_metrics() {
        let node = make_node("echo");

        node.announce_presence().await.unwrap();

        let m = node.metrics();
        assert_eq!(m.announcements_sent, 1);
        assert_eq!(m.messages_published, 1);
    }

    #[tokio::test]
    async fn announce_presence_includes_capabilities_and_topic() {
        let transport = InMemoryTransport::new();
        let alice = WakuA2ANode::new(
            "alice",
            "alice agent",
            vec!["summarize".into(), "code".into()],
            transport.clone(),
        );
        let bob = make_node_with_transport("bob", transport.clone());

        alice.announce_presence().await.unwrap();
        bob.poll_presence().await.unwrap();

        let peers = bob.peers().all_live();
        assert_eq!(peers[0].1.capabilities, vec!["summarize", "code"]);
        assert_eq!(peers[0].1.waku_topic, topics::task_topic(alice.pubkey()));
    }

    // --- poll_presence tests ---

    #[tokio::test]
    async fn poll_presence_ignores_self() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("self", transport.clone());

        node.announce_presence().await.unwrap();
        let count = node.poll_presence().await.unwrap();
        assert_eq!(count, 0);
        assert!(node.peers().all_live().is_empty());
    }

    #[tokio::test]
    async fn poll_presence_discovers_multiple_peers() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());
        let carol = make_node_with_transport("carol", transport.clone());
        let observer = make_node_with_transport("observer", transport.clone());

        alice.announce_presence().await.unwrap();
        bob.announce_presence().await.unwrap();
        carol.announce_presence().await.unwrap();

        let count = observer.poll_presence().await.unwrap();
        assert_eq!(count, 3);
        assert_eq!(observer.peers().all_live().len(), 3);
    }

    #[tokio::test]
    async fn poll_presence_increments_peers_discovered_metric() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        alice.announce_presence().await.unwrap();
        bob.poll_presence().await.unwrap();

        assert_eq!(bob.metrics().peers_discovered, 1);
    }

    #[tokio::test]
    async fn poll_presence_lazy_initializes_subscription() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        // First poll creates subscription, no messages yet
        let count1 = bob.poll_presence().await.unwrap();
        assert_eq!(count1, 0);

        // Alice announces after bob's subscription exists
        alice.announce_presence().await.unwrap();

        // Second poll reuses subscription and finds alice
        let count2 = bob.poll_presence().await.unwrap();
        assert_eq!(count2, 1);
    }

    #[tokio::test]
    async fn poll_presence_updates_existing_peer() {
        let transport = InMemoryTransport::new();
        let alice = make_node_with_transport("alice", transport.clone());
        let bob = make_node_with_transport("bob", transport.clone());

        alice.announce_presence_with_ttl(60).await.unwrap();
        bob.poll_presence().await.unwrap();

        let peers1 = bob.peers().all_live();
        assert_eq!(peers1[0].1.ttl_secs, 60);

        // Alice re-announces with different TTL
        alice.announce_presence_with_ttl(120).await.unwrap();
        bob.poll_presence().await.unwrap();

        let peers2 = bob.peers().all_live();
        assert_eq!(peers2.len(), 1);
        assert_eq!(peers2[0].1.ttl_secs, 120);
    }

    #[tokio::test]
    async fn poll_presence_returns_zero_when_no_messages() {
        let node = make_node("lonely");
        let count = node.poll_presence().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn poll_presence_ignores_non_presence_envelopes() {
        let transport = InMemoryTransport::new();
        let node = make_node_with_transport("observer", transport.clone());

        // Inject a Task envelope on the presence topic
        let task = logos_messaging_a2a_core::Task::new("from", "to", "hello");
        let envelope = A2AEnvelope::Task(task);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(topics::PRESENCE, &payload).await.unwrap();

        let count = node.poll_presence().await.unwrap();
        assert_eq!(count, 0);
    }

    // --- peers() / find_peers_by_capability() integration tests ---

    #[tokio::test]
    async fn peers_returns_empty_peer_map_initially() {
        let node = make_node("test");
        assert!(node.peers().is_empty());
        assert!(node.peers().all_live().is_empty());
    }

    #[tokio::test]
    async fn find_peers_by_capability_integration() {
        let transport = InMemoryTransport::new();
        let alice = WakuA2ANode::new(
            "alice",
            "alice agent",
            vec!["summarize".into(), "text".into()],
            transport.clone(),
        );
        let bob = WakuA2ANode::new("bob", "bob agent", vec!["code".into()], transport.clone());
        let observer = WakuA2ANode::new(
            "observer",
            "observer agent",
            vec!["observe".into()],
            transport.clone(),
        );

        alice.announce_presence().await.unwrap();
        bob.announce_presence().await.unwrap();
        observer.poll_presence().await.unwrap();

        let text_peers = observer.find_peers_by_capability("text");
        assert_eq!(text_peers.len(), 1);
        assert_eq!(text_peers[0].1.name, "alice");

        let code_peers = observer.find_peers_by_capability("code");
        assert_eq!(code_peers.len(), 1);
        assert_eq!(code_peers[0].1.name, "bob");

        let summarize_peers = observer.find_peers_by_capability("summarize");
        assert_eq!(summarize_peers.len(), 1);

        assert!(observer.find_peers_by_capability("nonexistent").is_empty());
    }

    // --- discover_all edge cases ---

    #[tokio::test]
    async fn discover_all_deduplicates_prefers_registry() {
        use logos_messaging_a2a_core::registry::{AgentRegistry, InMemoryRegistry};
        use std::sync::Arc;

        let transport = InMemoryTransport::new();
        let registry = Arc::new(InMemoryRegistry::new());

        let alice = make_node_with_transport("alice", transport.clone());
        let alice_pubkey = alice.card.public_key.clone();

        // Alice announces via Waku
        alice.announce().await.unwrap();

        // Also register alice in registry with updated name
        let reg_card = AgentCard {
            name: "alice-registry".into(),
            description: "from registry".into(),
            version: "1.0.0".into(),
            capabilities: vec!["test".into()],
            public_key: alice_pubkey.clone(),
            intro_bundle: None,
        };
        registry.register(reg_card).await.unwrap();

        let discoverer = make_node_with_transport("discoverer", transport.clone());
        let discoverer = discoverer.with_registry(registry);

        let all = discoverer.discover_all().await.unwrap();
        assert_eq!(all.len(), 1);
        // Registry version should win (overwrites Waku)
        assert_eq!(all[0].name, "alice-registry");
    }

    // --- announce + discover roundtrip ---

    #[tokio::test]
    async fn announce_discover_roundtrip_preserves_card_fields() {
        let transport = InMemoryTransport::new();
        let alice = WakuA2ANode::new(
            "alice",
            "Alice the summarizer",
            vec!["summarize".into(), "text".into()],
            transport.clone(),
        );
        let bob = make_node_with_transport("bob", transport.clone());

        alice.announce().await.unwrap();
        let cards = bob.discover().await.unwrap();

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "alice");
        assert_eq!(cards[0].description, "Alice the summarizer");
        assert_eq!(cards[0].capabilities, vec!["summarize", "text"]);
        assert_eq!(cards[0].public_key, alice.card.public_key);
    }

    // --- full lifecycle ---

    #[tokio::test]
    async fn full_discovery_lifecycle() {
        let transport = InMemoryTransport::new();
        let alice = WakuA2ANode::new(
            "alice",
            "alice agent",
            vec!["text".into()],
            transport.clone(),
        );
        let bob = WakuA2ANode::new("bob", "bob agent", vec!["code".into()], transport.clone());

        // Step 1: Announce agent cards
        alice.announce().await.unwrap();
        bob.announce().await.unwrap();

        // Step 2: Each discovers the other
        let alice_found = alice.discover().await.unwrap();
        assert_eq!(alice_found.len(), 1);
        assert_eq!(alice_found[0].name, "bob");

        let bob_found = bob.discover().await.unwrap();
        assert_eq!(bob_found.len(), 1);
        assert_eq!(bob_found[0].name, "alice");

        // Step 3: Announce presence
        alice.announce_presence().await.unwrap();
        bob.announce_presence().await.unwrap();

        // Step 4: Poll presence
        let alice_peers = alice.poll_presence().await.unwrap();
        assert_eq!(alice_peers, 1);
        assert_eq!(alice.peers().all_live().len(), 1);
        assert_eq!(alice.peers().all_live()[0].1.name, "bob");

        let bob_peers = bob.poll_presence().await.unwrap();
        assert_eq!(bob_peers, 1);
        assert_eq!(bob.peers().all_live()[0].1.name, "alice");

        // Step 5: Verify capability search
        let text_peers = bob.find_peers_by_capability("text");
        assert_eq!(text_peers.len(), 1);
        assert_eq!(text_peers[0].1.name, "alice");
    }
}
