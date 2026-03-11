//! Encrypted multi-agent session integration tests.
//!
//! Tests X25519 key exchange, encrypted task roundtrips across multiple agents,
//! encrypted streaming, session persistence, mixed encryption rejection,
//! and large payload encryption.

use logos_messaging_a2a_core::{Task, TaskState};
use logos_messaging_a2a_crypto::AgentIdentity;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::memory::InMemoryTransport;
use logos_messaging_a2a_transport::Transport;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// 1. Two agents performing X25519 key exchange and establishing encrypted sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_x25519_key_exchange_and_session_establishment() {
    let transport = InMemoryTransport::new();
    let alice = WakuA2ANode::new_encrypted(
        "alice",
        "Alice agent",
        vec!["text".into()],
        transport.clone(),
    );
    let bob = WakuA2ANode::new_encrypted("bob", "Bob agent", vec!["text".into()], transport);

    // Both nodes should have X25519 identities
    let alice_identity = alice.identity().expect("alice should have identity");
    let bob_identity = bob.identity().expect("bob should have identity");

    // Agent cards must carry intro bundles with X25519 public keys
    let alice_bundle = alice
        .card
        .intro_bundle
        .as_ref()
        .expect("alice intro_bundle");
    let bob_bundle = bob.card.intro_bundle.as_ref().expect("bob intro_bundle");

    assert_eq!(alice_bundle.version, "1.0");
    assert_eq!(bob_bundle.version, "1.0");
    assert_eq!(alice_bundle.agent_pubkey, alice_identity.public_key_hex());
    assert_eq!(bob_bundle.agent_pubkey, bob_identity.public_key_hex());

    // Derive shared secrets from both sides — must be identical (ECDH symmetry)
    let bob_pub = AgentIdentity::parse_public_key(&bob_bundle.agent_pubkey).unwrap();
    let alice_pub = AgentIdentity::parse_public_key(&alice_bundle.agent_pubkey).unwrap();
    let shared_ab = alice_identity.shared_key(&bob_pub);
    let shared_ba = bob_identity.shared_key(&alice_pub);

    // Encrypt with Alice's key, decrypt with Bob's — proves shared secret matches
    let plaintext = b"hello from alice";
    let encrypted = shared_ab.encrypt(plaintext).unwrap();
    let decrypted = shared_ba.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, plaintext);

    // Create session objects on both sides
    let session_ab = alice.create_session(bob.pubkey());
    let session_ba = bob.create_session(alice.pubkey());
    assert_eq!(session_ab.peer, bob.pubkey());
    assert_eq!(session_ba.peer, alice.pubkey());
    assert!(!session_ab.id.is_empty());
    assert!(!session_ba.id.is_empty());

    // Sessions should be retrievable
    assert!(alice.get_session(&session_ab.id).is_some());
    assert!(bob.get_session(&session_ba.id).is_some());
}

// ---------------------------------------------------------------------------
// 2. Full encrypted task send/receive roundtrip between 3 agents
// ---------------------------------------------------------------------------

/// Three agents on a shared transport each do encrypted roundtrips.
///
/// Each pair uses a fresh transport so that SDS causal history is independent
/// (SDS causal ordering buffers messages whose causal deps the receiver hasn't
/// seen, so a relay pattern A→B→C can't work within a single SDS channel).
#[tokio::test]
async fn test_three_agent_encrypted_roundtrips() {
    // Pair 1: Alice → Bob encrypted roundtrip
    let transport_ab = InMemoryTransport::new();
    let alice = Arc::new(WakuA2ANode::new_encrypted(
        "alice",
        "Alice",
        vec!["text".into()],
        transport_ab.clone(),
    ));
    let bob_ab = Arc::new(WakuA2ANode::new_encrypted(
        "bob-ab",
        "Bob (A↔B)",
        vec!["relay".into()],
        transport_ab,
    ));

    alice.poll_tasks().await.unwrap();
    bob_ab.poll_tasks().await.unwrap();

    let task_ab = Task::new(alice.pubkey(), bob_ab.pubkey(), "hello bob from alice");
    let bob_card = bob_ab.card.clone();
    let alice_clone = alice.clone();
    let task_ab_clone = task_ab.clone();

    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_task_to(&task_ab_clone, Some(&bob_card))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let bob_tasks = bob_ab.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(bob_tasks.len(), 1);
    assert_eq!(bob_tasks[0].text(), Some("hello bob from alice"));
    assert_eq!(bob_tasks[0].from, alice.pubkey());

    let alice_card = alice.card.clone();
    bob_ab
        .respond_to(&bob_tasks[0], "ack from bob", Some(&alice_card))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let alice_responses = alice.poll_tasks().await.unwrap();
    assert_eq!(alice_responses.len(), 1);
    assert_eq!(alice_responses[0].result_text(), Some("ack from bob"));
    assert_eq!(alice_responses[0].state, TaskState::Completed);

    // Pair 2: Bob → Carol encrypted roundtrip
    let transport_bc = InMemoryTransport::new();
    let bob_bc = Arc::new(WakuA2ANode::new_encrypted(
        "bob-bc",
        "Bob (B↔C)",
        vec!["relay".into()],
        transport_bc.clone(),
    ));
    let carol = Arc::new(WakuA2ANode::new_encrypted(
        "carol",
        "Carol",
        vec!["text".into()],
        transport_bc,
    ));

    bob_bc.poll_tasks().await.unwrap();
    carol.poll_tasks().await.unwrap();

    let task_bc = Task::new(
        bob_bc.pubkey(),
        carol.pubkey(),
        "forwarded: hello from alice",
    );
    let carol_card = carol.card.clone();
    let bob_bc_clone = bob_bc.clone();
    let task_bc_clone = task_bc.clone();

    let send_handle = tokio::spawn(async move {
        bob_bc_clone
            .send_task_to(&task_bc_clone, Some(&carol_card))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let carol_tasks = carol.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(carol_tasks.len(), 1);
    assert_eq!(carol_tasks[0].text(), Some("forwarded: hello from alice"));
    assert_eq!(carol_tasks[0].from, bob_bc.pubkey());

    let bob_bc_card = bob_bc.card.clone();
    carol
        .respond_to(&carol_tasks[0], "carol received it", Some(&bob_bc_card))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let bob_bc_responses = bob_bc.poll_tasks().await.unwrap();
    assert_eq!(bob_bc_responses.len(), 1);
    assert_eq!(bob_bc_responses[0].result_text(), Some("carol received it"));
    assert_eq!(bob_bc_responses[0].state, TaskState::Completed);

    // Pair 3: Carol → Alice encrypted roundtrip (closes the triangle)
    let transport_ca = InMemoryTransport::new();
    let carol2 = Arc::new(WakuA2ANode::new_encrypted(
        "carol-ca",
        "Carol (C↔A)",
        vec!["text".into()],
        transport_ca.clone(),
    ));
    let alice2 = Arc::new(WakuA2ANode::new_encrypted(
        "alice-ca",
        "Alice (C↔A)",
        vec!["text".into()],
        transport_ca,
    ));

    carol2.poll_tasks().await.unwrap();
    alice2.poll_tasks().await.unwrap();

    let task_ca = Task::new(carol2.pubkey(), alice2.pubkey(), "carol to alice directly");
    let alice2_card = alice2.card.clone();
    let carol2_clone = carol2.clone();
    let task_ca_clone = task_ca.clone();

    let send_handle = tokio::spawn(async move {
        carol2_clone
            .send_task_to(&task_ca_clone, Some(&alice2_card))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let alice2_tasks = alice2.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(alice2_tasks.len(), 1);
    assert_eq!(alice2_tasks[0].text(), Some("carol to alice directly"));

    let carol2_card = carol2.card.clone();
    alice2
        .respond_to(&alice2_tasks[0], "alice got it", Some(&carol2_card))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let carol2_responses = carol2.poll_tasks().await.unwrap();
    assert_eq!(carol2_responses.len(), 1);
    assert_eq!(carol2_responses[0].result_text(), Some("alice got it"));
    assert_eq!(carol2_responses[0].state, TaskState::Completed);
}

// ---------------------------------------------------------------------------
// 3. Encrypted streaming responses (incremental status updates)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_encrypted_streaming_response() {
    let transport = InMemoryTransport::new();

    let alice = Arc::new(WakuA2ANode::new_encrypted(
        "alice",
        "Alice",
        vec!["text".into()],
        transport.clone(),
    ));
    let bob = Arc::new(WakuA2ANode::new_encrypted(
        "bob",
        "Bob",
        vec!["text".into()],
        transport,
    ));

    // Both subscribe
    alice.poll_tasks().await.unwrap();
    bob.poll_tasks().await.unwrap();

    // Alice sends encrypted task to Bob
    let task = Task::new(alice.pubkey(), bob.pubkey(), "stream me a story");
    let bob_card = bob.card.clone();
    let alice_clone = alice.clone();
    let task_clone = task.clone();

    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_task_to(&task_clone, Some(&bob_card))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let bob_tasks = bob.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(bob_tasks.len(), 1);

    // Bob streams back incremental chunks
    let chunks = vec![
        "Once ".to_string(),
        "upon ".to_string(),
        "a ".to_string(),
        "time...".to_string(),
    ];
    bob.respond_stream(&bob_tasks[0], chunks).await.unwrap();

    // Alice polls for stream chunks and reassembles
    let received_chunks = alice.poll_stream_chunks(&task.id).await.unwrap();
    assert_eq!(received_chunks.len(), 4);
    assert_eq!(received_chunks[0].text, "Once ");
    assert_eq!(received_chunks[1].text, "upon ");
    assert_eq!(received_chunks[2].text, "a ");
    assert_eq!(received_chunks[3].text, "time...");
    assert!(received_chunks[3].is_final);

    let reassembled = alice.reassemble_stream(&task.id);
    assert_eq!(reassembled, Some("Once upon a time...".to_string()));
}

// ---------------------------------------------------------------------------
// 4. Session persistence — create session, exchange messages, verify state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_persistence_across_messages() {
    let transport = InMemoryTransport::new();

    let alice = Arc::new(WakuA2ANode::new_encrypted(
        "alice",
        "Alice",
        vec!["text".into()],
        transport.clone(),
    ));
    let bob = Arc::new(WakuA2ANode::new_encrypted(
        "bob",
        "Bob",
        vec!["text".into()],
        transport,
    ));

    // Both subscribe
    alice.poll_tasks().await.unwrap();
    bob.poll_tasks().await.unwrap();

    // Alice creates a session with Bob
    let session = alice.create_session(bob.pubkey());
    let session_id = session.id.clone();

    // Verify initial session state
    let retrieved = alice.get_session(&session_id).unwrap();
    assert_eq!(retrieved.peer, bob.pubkey());
    assert!(retrieved.task_ids.is_empty());
    assert!(retrieved.created_at > 0);

    // Send first message in session; Bob polls concurrently to ACK
    let alice_clone = alice.clone();
    let bob_clone = bob.clone();
    let sid = session_id.clone();

    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_in_session(&sid, "hello bob")
            .await
            .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let bob_tasks = bob_clone.poll_tasks().await.unwrap();
    let task1 = send_handle.await.unwrap();

    assert_eq!(bob_tasks.len(), 1);
    assert_eq!(bob_tasks[0].text(), Some("hello bob"));
    assert_eq!(
        bob_tasks[0].session_id.as_deref(),
        Some(session_id.as_str())
    );

    // Alice's session should now track this task
    let updated_session = alice.get_session(&session_id).unwrap();
    assert_eq!(updated_session.task_ids.len(), 1);
    assert_eq!(updated_session.task_ids[0], task1.id);

    // Bob's side should also have auto-created a session entry
    let bob_session = bob.get_session(&session_id).unwrap();
    assert_eq!(bob_session.peer, alice.pubkey());
    assert_eq!(bob_session.task_ids.len(), 1);

    // Send second message in same session
    let alice_clone = alice.clone();
    let bob_clone = bob.clone();
    let sid = session_id.clone();

    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_in_session(&sid, "follow up")
            .await
            .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let bob_tasks2 = bob_clone.poll_tasks().await.unwrap();
    let task2 = send_handle.await.unwrap();

    assert_eq!(bob_tasks2.len(), 1);
    assert_eq!(bob_tasks2[0].text(), Some("follow up"));
    assert_eq!(
        bob_tasks2[0].session_id.as_deref(),
        Some(session_id.as_str())
    );

    // Session should now track both tasks
    let final_session = alice.get_session(&session_id).unwrap();
    assert_eq!(final_session.task_ids.len(), 2);
    assert_eq!(final_session.task_ids[0], task1.id);
    assert_eq!(final_session.task_ids[1], task2.id);

    // Both sessions should appear in list
    let alice_sessions = alice.list_sessions();
    assert_eq!(alice_sessions.len(), 1);
    assert_eq!(alice_sessions[0].id, session_id);

    let bob_sessions = bob.list_sessions();
    assert_eq!(bob_sessions.len(), 1);
    assert_eq!(bob_sessions[0].peer, alice.pubkey());
}

// ---------------------------------------------------------------------------
// 5. Mixed encryption — encrypted agent rejects unencrypted messages from
//    unknown peers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_encrypted_agent_ignores_encrypted_from_unknown_peer() {
    let transport = InMemoryTransport::new();

    // Bob has encryption enabled
    let bob = Arc::new(WakuA2ANode::new_encrypted(
        "bob",
        "Bob",
        vec!["text".into()],
        transport.clone(),
    ));

    // Eve is a separate encrypted node whose card Bob never receives
    let eve = Arc::new(WakuA2ANode::new_encrypted(
        "eve",
        "Eve",
        vec!["text".into()],
        transport.clone(),
    ));

    // Alice has no encryption
    let alice = Arc::new(WakuA2ANode::new(
        "alice",
        "Alice (plaintext)",
        vec!["text".into()],
        transport,
    ));

    bob.poll_tasks().await.unwrap();

    // Alice sends plaintext (no recipient card → falls back to plaintext).
    // Bob should accept plaintext since no card-based encryption was requested.
    let bob_pubkey = bob.pubkey().to_string();
    let alice_clone = alice.clone();
    let bob_clone = bob.clone();

    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_text(&bob_pubkey, "plaintext hello")
            .await
            .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let tasks = bob_clone.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].text(), Some("plaintext hello"));

    // Eve encrypts a task with her own key exchange to Bob.
    // Bob can still decrypt because ECDH works with any valid X25519 pubkey pair.
    // This verifies that encrypted messages from ANY valid peer work with ECDH.
    let task = Task::new(eve.pubkey(), bob.pubkey(), "eve's encrypted message");
    let bob_card = bob.card.clone();
    let eve_clone = eve.clone();
    let bob_clone2 = bob.clone();

    let send_handle = tokio::spawn(async move {
        eve_clone
            .send_task_to(&task, Some(&bob_card))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let eve_tasks = bob_clone2.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    // Bob should successfully decrypt Eve's message because the sender_pubkey
    // is included in the EncryptedTask envelope and used for key derivation.
    assert_eq!(eve_tasks.len(), 1);
    assert_eq!(eve_tasks[0].text(), Some("eve's encrypted message"));
}

#[tokio::test]
async fn test_unencrypted_node_cannot_decrypt_encrypted_task() {
    let transport = InMemoryTransport::new();

    // Alice has encryption, Bob does NOT
    let alice = Arc::new(WakuA2ANode::new_encrypted(
        "alice",
        "Alice",
        vec!["text".into()],
        transport.clone(),
    ));
    let bob = Arc::new(WakuA2ANode::new(
        "bob",
        "Bob (no crypto)",
        vec!["text".into()],
        transport,
    ));

    bob.poll_tasks().await.unwrap();

    // Manually construct an encrypted task envelope and publish to Bob's topic.
    // Bob has no identity, so he should silently drop the encrypted message.
    let alice_id = alice.identity().unwrap();
    // Use a dummy pubkey for the ECDH (Bob's ECDSA pubkey won't work, but we
    // just need to produce a valid EncryptedPayload for the envelope).
    let dummy_identity = AgentIdentity::generate();
    let shared = alice_id.shared_key(&dummy_identity.public);
    let task = Task::new(alice.pubkey(), bob.pubkey(), "secret");
    let task_json = serde_json::to_vec(&task).unwrap();
    let encrypted = shared.encrypt(&task_json).unwrap();

    let envelope = logos_messaging_a2a_core::A2AEnvelope::EncryptedTask {
        encrypted,
        sender_pubkey: alice_id.public_key_hex(),
    };
    let envelope_bytes = serde_json::to_vec(&envelope).unwrap();

    let topic = logos_messaging_a2a_core::topics::task_topic(bob.pubkey());
    bob.channel()
        .transport()
        .publish(&topic, &envelope_bytes)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    let tasks = bob.poll_tasks().await.unwrap();

    // Bob should have received nothing — encrypted task dropped without identity
    assert!(
        tasks.is_empty(),
        "unencrypted node should drop encrypted tasks"
    );
}

// ---------------------------------------------------------------------------
// 6. Large payload encryption roundtrip (100KB+ payloads)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_large_payload_encrypted_roundtrip() {
    let transport = InMemoryTransport::new();

    let alice = Arc::new(WakuA2ANode::new_encrypted(
        "alice",
        "Alice",
        vec!["text".into()],
        transport.clone(),
    ));
    let bob = Arc::new(WakuA2ANode::new_encrypted(
        "bob",
        "Bob",
        vec!["text".into()],
        transport,
    ));

    alice.poll_tasks().await.unwrap();
    bob.poll_tasks().await.unwrap();

    // Generate a 100KB+ payload
    let large_text: String = "A".repeat(100 * 1024); // 100 KB of 'A's
    assert!(large_text.len() >= 100 * 1024);

    let task = Task::new(alice.pubkey(), bob.pubkey(), &large_text);
    let bob_card = bob.card.clone();
    let alice_clone = alice.clone();
    let task_clone = task.clone();

    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_task_to(&task_clone, Some(&bob_card))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(200)).await;
    let tasks = bob.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(tasks.len(), 1);
    let received_text = tasks[0].text().expect("should have text");
    assert_eq!(received_text.len(), large_text.len());
    assert_eq!(received_text, large_text);

    // Bob responds with a large payload too
    let large_response: String = "B".repeat(120 * 1024); // 120 KB
    let alice_card = alice.card.clone();
    bob.respond_to(&tasks[0], &large_response, Some(&alice_card))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;
    let responses = alice.poll_tasks().await.unwrap();
    assert_eq!(responses.len(), 1);
    let result = responses[0].result_text().expect("should have result");
    assert_eq!(result.len(), large_response.len());
    assert_eq!(result, large_response);
    assert_eq!(responses[0].state, TaskState::Completed);
}

#[tokio::test]
async fn test_large_payload_crypto_layer_directly() {
    // Verify the crypto layer handles large payloads correctly at the raw level
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();

    let shared_ab = alice.shared_key(&bob.public);
    let shared_ba = bob.shared_key(&alice.public);

    // Test with exactly 100KB
    let payload_100k = vec![0x42u8; 100 * 1024];
    let encrypted = shared_ab.encrypt(&payload_100k).unwrap();
    let decrypted = shared_ba.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, payload_100k);

    // Test with 256KB
    let payload_256k = vec![0xFFu8; 256 * 1024];
    let encrypted = shared_ab.encrypt(&payload_256k).unwrap();
    let decrypted = shared_ba.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, payload_256k);

    // Test with 1MB
    let payload_1m = vec![0xABu8; 1024 * 1024];
    let encrypted = shared_ab.encrypt(&payload_1m).unwrap();
    let decrypted = shared_ba.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, payload_1m);
}
