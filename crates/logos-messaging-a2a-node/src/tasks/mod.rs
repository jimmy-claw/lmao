//! Task sending, receiving, and payment operations for [`WakuA2ANode`](crate::WakuA2ANode).

mod helpers;
mod payment;
mod poll;
mod respond;
mod send;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use crate::tasks::test_support::{fast_config, MockTransport};
    use crate::WakuA2ANode;
    use k256::ecdsa::SigningKey;
    use logos_messaging_a2a_core::{topics, A2AEnvelope, AgentCard, PresenceAnnouncement};

    fn rand_core() -> k256::elliptic_curve::rand_core::OsRng {
        k256::elliptic_curve::rand_core::OsRng
    }

    #[test]
    fn test_node_creation() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec!["text".into()], transport);
        assert_eq!(node.card.name, "test");
        assert!(!node.pubkey().is_empty());
        assert_eq!(node.pubkey().len(), 66);
        assert!(node.identity().is_none());
        assert!(node.card.intro_bundle.is_none());
    }

    #[test]
    fn test_encrypted_node_creation() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new_encrypted("test", "test agent", vec!["text".into()], transport);
        assert!(node.identity().is_some());
        assert!(node.card.intro_bundle.is_some());
        let bundle = node.card.intro_bundle.as_ref().unwrap();
        assert_eq!(bundle.version, "1.0");
        assert_eq!(bundle.agent_pubkey.len(), 64);
    }

    #[tokio::test]
    async fn test_announce() {
        let transport = MockTransport::new();
        let published = transport.published.clone();
        let node = WakuA2ANode::new("echo", "echo agent", vec!["text".into()], transport);

        node.announce().await.unwrap();

        let msgs = published.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, topics::DISCOVERY);

        let envelope: A2AEnvelope = serde_json::from_slice(&msgs[0].1).unwrap();
        match envelope {
            A2AEnvelope::AgentCard(card) => {
                assert_eq!(card.name, "echo");
                assert_eq!(card.public_key, node.pubkey());
            }
            _ => panic!("Expected AgentCard envelope"),
        }
    }

    #[tokio::test]
    async fn test_discover() {
        let transport = MockTransport::new();
        let other_card = AgentCard {
            name: "other".to_string(),
            description: "other agent".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["code".to_string()],
            public_key: "02deadbeef".to_string(),
            intro_bundle: None,
        };
        let envelope = A2AEnvelope::AgentCard(other_card.clone());
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.inject(topics::DISCOVERY, payload);

        let node = WakuA2ANode::new("me", "my agent", vec![], transport);
        let cards = node.discover().await.unwrap();

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "other");
    }

    #[tokio::test]
    async fn test_channel_accessible() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        assert_eq!(node.channel().sender_id(), node.pubkey());
        assert_eq!(node.channel().outgoing_pending(), 0);
        assert_eq!(node.channel().incoming_pending(), 0);
    }

    #[test]
    fn test_create_session() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        let session = node.create_session("02deadbeef");
        assert_eq!(session.peer, "02deadbeef");
        assert!(session.task_ids.is_empty());
        assert!(!session.id.is_empty());
    }

    #[test]
    fn test_list_sessions() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        assert!(node.list_sessions().is_empty());
        node.create_session("02aa");
        node.create_session("02bb");
        assert_eq!(node.list_sessions().len(), 2);
    }

    #[test]
    fn test_get_session() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec![], transport);
        let session = node.create_session("02aa");
        let found = node.get_session(&session.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().peer, "02aa");
        assert!(node.get_session("nonexistent").is_none());
    }

    #[test]
    fn test_from_key_deterministic_pubkey() {
        let transport = MockTransport::new();
        let key = SigningKey::random(&mut rand_core());
        let expected_pk = hex::encode(key.verifying_key().to_encoded_point(true).as_bytes());

        let node = WakuA2ANode::from_key(
            "det",
            "deterministic node",
            vec!["text".into()],
            transport,
            key,
        );
        assert_eq!(node.pubkey(), expected_pk);
        assert_eq!(node.card.name, "det");
        assert_eq!(node.card.description, "deterministic node");
        assert_eq!(node.card.capabilities, vec!["text".to_string()]);
    }

    #[test]
    fn test_from_key_same_key_same_pubkey() {
        let key_bytes = [42u8; 32];
        let key1 = SigningKey::from_bytes((&key_bytes).into()).unwrap();
        let key2 = SigningKey::from_bytes((&key_bytes).into()).unwrap();

        let node1 = WakuA2ANode::from_key("a", "a", vec![], MockTransport::new(), key1);
        let node2 = WakuA2ANode::from_key("b", "b", vec![], MockTransport::new(), key2);

        assert_eq!(node1.pubkey(), node2.pubkey());
    }

    #[test]
    fn test_with_config_custom_settings() {
        let transport = MockTransport::new();
        let config = logos_messaging_a2a_transport::sds::ChannelConfig {
            ack_timeout: std::time::Duration::from_secs(30),
            max_retries: 5,
            ..Default::default()
        };
        let node = WakuA2ANode::with_config(
            "configured",
            "custom config node",
            vec!["image".into()],
            transport,
            config,
        );
        assert_eq!(node.card.name, "configured");
        assert_eq!(node.card.capabilities, vec!["image".to_string()]);
        assert!(!node.pubkey().is_empty());
    }

    #[test]
    fn test_find_peers_by_capability() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        node.peer_map.update(&PresenceAnnouncement {
            agent_id: "peer1".into(),
            name: "img-agent".into(),
            capabilities: vec!["image".into(), "text".into()],
            waku_topic: "/lmao/1/tasks/peer1/proto".into(),
            ttl_secs: 300,
            signature: None,
        });
        node.peer_map.update(&PresenceAnnouncement {
            agent_id: "peer2".into(),
            name: "txt-agent".into(),
            capabilities: vec!["text".into()],
            waku_topic: "/lmao/1/tasks/peer2/proto".into(),
            ttl_secs: 300,
            signature: None,
        });

        assert_eq!(node.find_peers_by_capability("image").len(), 1);
        assert_eq!(node.find_peers_by_capability("text").len(), 2);
        assert_eq!(node.find_peers_by_capability("video").len(), 0);
    }

    #[tokio::test]
    async fn test_announce_presence_and_poll() {
        let transport = MockTransport::new();

        let alice = WakuA2ANode::with_config(
            "alice",
            "Alice agent",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let bob = WakuA2ANode::with_config(
            "bob",
            "Bob agent",
            vec!["code".into()],
            transport.clone(),
            fast_config(),
        );

        alice.announce_presence().await.unwrap();

        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 1);

        let peers = bob.peers().all_live();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].1.name, "alice");
    }

    #[test]
    fn test_peer_map_default_trait() {
        let map = crate::presence::PeerMap::default();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[tokio::test]
    async fn test_presence_self_not_in_peers() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config(
            "self",
            "self agent",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );

        // Announce and poll own presence
        node.announce_presence().await.unwrap();
        let count = node.poll_presence().await.unwrap();
        assert_eq!(
            count, 0,
            "node should ignore its own presence announcements"
        );
    }

    #[tokio::test]
    async fn test_announce_presence_with_ttl() {
        let transport = MockTransport::new();
        let alice = WakuA2ANode::with_config(
            "alice",
            "alice agent",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let bob =
            WakuA2ANode::with_config("bob", "bob agent", vec![], transport.clone(), fast_config());

        alice.announce_presence_with_ttl(600).await.unwrap();

        let count = bob.poll_presence().await.unwrap();
        assert_eq!(count, 1);

        let peers = bob.peers().all_live();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].1.ttl_secs, 600);
    }

    #[test]
    fn test_pubkey_is_valid_hex() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        // Should be valid hex
        let decoded = hex::decode(node.pubkey());
        assert!(decoded.is_ok(), "pubkey should be valid hex");
        // Compressed secp256k1 key is 33 bytes
        assert_eq!(decoded.unwrap().len(), 33);
    }

    #[test]
    fn test_card_version_is_set() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test agent", vec!["cap".into()], transport);
        assert_eq!(node.card.version, "0.1.0");
    }

    #[test]
    fn test_card_capabilities_match() {
        let transport = MockTransport::new();
        let caps = vec!["text".to_string(), "image".to_string(), "code".to_string()];
        let node = WakuA2ANode::new("test", "test agent", caps.clone(), transport);
        assert_eq!(node.card.capabilities, caps);
    }

    #[tokio::test]
    async fn test_discover_excludes_self() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("self", "self agent", vec![], transport.clone());

        // Announce self
        node.announce().await.unwrap();

        // Discover should not include self
        let cards = node.discover().await.unwrap();
        assert!(cards.is_empty(), "discover should exclude own card");
    }

    #[tokio::test]
    async fn test_discover_returns_multiple_cards() {
        let transport = MockTransport::new();

        // Inject three agent cards
        for i in 0..3 {
            let card = AgentCard {
                name: format!("agent-{i}"),
                description: format!("agent {i}"),
                version: "0.1.0".to_string(),
                capabilities: vec![],
                public_key: format!("02{i:064x}"),
                intro_bundle: None,
            };
            let envelope = A2AEnvelope::AgentCard(card);
            let payload = serde_json::to_vec(&envelope).unwrap();
            transport.inject(topics::DISCOVERY, payload);
        }

        let node = WakuA2ANode::new("me", "me", vec![], transport);
        let cards = node.discover().await.unwrap();
        assert_eq!(cards.len(), 3);
        assert!(cards.iter().any(|c| c.name == "agent-0"));
        assert!(cards.iter().any(|c| c.name == "agent-1"));
        assert!(cards.iter().any(|c| c.name == "agent-2"));
    }

    #[tokio::test]
    async fn test_poll_presence_multiple_peers() {
        let transport = MockTransport::new();
        let node1 = WakuA2ANode::with_config(
            "node1",
            "node1",
            vec!["text".into()],
            transport.clone(),
            fast_config(),
        );
        let node2 = WakuA2ANode::with_config(
            "node2",
            "node2",
            vec!["code".into()],
            transport.clone(),
            fast_config(),
        );
        let observer = WakuA2ANode::with_config(
            "observer",
            "observer",
            vec![],
            transport.clone(),
            fast_config(),
        );

        node1.announce_presence().await.unwrap();
        node2.announce_presence().await.unwrap();

        let count = observer.poll_presence().await.unwrap();
        assert_eq!(count, 2);

        let peers = observer.peers().all_live();
        assert_eq!(peers.len(), 2);
    }

    #[test]
    fn test_with_retry_builder() {
        let transport = MockTransport::new();
        let config = logos_messaging_a2a_core::RetryConfig {
            max_attempts: 3,
            base_delay_ms: 100,
            max_delay_ms: 5000,
            jitter: false,
        };
        let node = WakuA2ANode::new("test", "test", vec![], transport).with_retry(config);
        assert!(node.retry_config().is_some());
        let cfg = node.retry_config().unwrap();
        assert_eq!(cfg.max_attempts, 3);
        assert_eq!(cfg.base_delay_ms, 100);
        assert_eq!(cfg.max_delay_ms, 5000);
        assert!(!cfg.jitter);
    }

    #[test]
    fn test_retry_config_none_by_default() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert!(node.retry_config().is_none());
    }

    #[test]
    fn test_signing_key_accessor() {
        let transport = MockTransport::new();
        let key = SigningKey::random(&mut rand_core());
        let expected_pk = hex::encode(key.verifying_key().to_encoded_point(true).as_bytes());
        let node = WakuA2ANode::from_key("test", "test", vec![], transport, key);
        // signing_key should produce the same pubkey
        let sk_pk = hex::encode(
            node.signing_key()
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        );
        assert_eq!(sk_pk, expected_pk);
    }

    #[test]
    fn test_identity_none_for_unencrypted_node() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert!(node.identity().is_none());
    }

    #[test]
    fn test_identity_some_for_encrypted_node() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new_encrypted("test", "test", vec![], transport);
        assert!(node.identity().is_some());
    }

    #[tokio::test]
    async fn test_channel_sender_id_matches_pubkey() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert_eq!(node.channel().sender_id(), node.pubkey());
    }
}

#[cfg(test)]
mod session_tests {
    use crate::tasks::test_support::{fast_config, MockTransport};
    use crate::WakuA2ANode;

    #[test]
    fn session_has_unique_uuid() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let s1 = node.create_session("peer-a");
        let s2 = node.create_session("peer-b");
        assert_ne!(s1.id, s2.id, "sessions should have unique IDs");
    }

    #[test]
    fn session_created_at_is_recent() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let session = node.create_session("peer-a");
        // created_at should be within 2 seconds of now
        assert!(
            session.created_at >= now - 2 && session.created_at <= now + 2,
            "session created_at should be close to current time"
        );
    }

    #[test]
    fn session_starts_with_empty_task_ids() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let session = node.create_session("peer-a");
        assert!(session.task_ids.is_empty());
    }

    #[test]
    fn session_peer_preserved_correctly() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let long_key = "02".to_string() + &"ab".repeat(32);
        let session = node.create_session(&long_key);
        assert_eq!(session.peer, long_key);
    }

    #[tokio::test]
    async fn multiple_tasks_tracked_in_session() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config());
        let session = node.create_session("02deadbeef");

        // Send multiple messages in the same session
        let t1 = node.send_in_session(&session.id, "first").await.unwrap();
        let t2 = node.send_in_session(&session.id, "second").await.unwrap();
        let t3 = node.send_in_session(&session.id, "third").await.unwrap();

        let updated = node.get_session(&session.id).unwrap();
        assert_eq!(updated.task_ids.len(), 3);
        assert_eq!(updated.task_ids[0], t1.id);
        assert_eq!(updated.task_ids[1], t2.id);
        assert_eq!(updated.task_ids[2], t3.id);
    }

    #[tokio::test]
    async fn tasks_in_session_carry_session_id() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config());
        let session = node.create_session("02deadbeef");

        let task = node.send_in_session(&session.id, "hello").await.unwrap();
        assert_eq!(task.session_id, Some(session.id.clone()));
    }

    #[test]
    fn sessions_isolated_between_peers() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let s1 = node.create_session("peer-a");
        let s2 = node.create_session("peer-b");

        assert_ne!(s1.id, s2.id);
        assert_eq!(s1.peer, "peer-a");
        assert_eq!(s2.peer, "peer-b");

        // Getting one doesn't affect the other
        let got1 = node.get_session(&s1.id).unwrap();
        let got2 = node.get_session(&s2.id).unwrap();
        assert_eq!(got1.peer, "peer-a");
        assert_eq!(got2.peer, "peer-b");
    }

    #[test]
    fn multiple_sessions_with_same_peer() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let s1 = node.create_session("peer-a");
        let s2 = node.create_session("peer-a");

        // Two distinct sessions, same peer
        assert_ne!(s1.id, s2.id);
        assert_eq!(s1.peer, s2.peer);
        assert_eq!(node.list_sessions().len(), 2);
    }

    #[test]
    fn get_session_returns_clone() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        let session = node.create_session("peer-a");

        let got1 = node.get_session(&session.id).unwrap();
        let got2 = node.get_session(&session.id).unwrap();
        assert_eq!(got1.id, got2.id);
        assert_eq!(got1.peer, got2.peer);
    }

    #[test]
    fn list_sessions_empty_initially() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);
        assert!(node.list_sessions().is_empty());
    }

    #[test]
    fn list_sessions_contains_all_created() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let ids: Vec<String> = (0..5)
            .map(|i| node.create_session(&format!("peer-{i}")).id)
            .collect();

        let sessions = node.list_sessions();
        assert_eq!(sessions.len(), 5);
        for id in &ids {
            assert!(sessions.iter().any(|s| &s.id == id));
        }
    }
}
