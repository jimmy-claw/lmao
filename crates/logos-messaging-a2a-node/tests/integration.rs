//! Integration tests: full agent-to-agent task lifecycle using InMemoryTransport.

use logos_messaging_a2a_core::{Task, TaskState};
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::memory::InMemoryTransport;
use std::sync::Arc;
use std::time::Duration;

/// Helper: create a node pair sharing one transport.
fn make_arc_pair() -> (
    Arc<WakuA2ANode<InMemoryTransport>>,
    Arc<WakuA2ANode<InMemoryTransport>>,
) {
    let transport = InMemoryTransport::new();
    let alice = Arc::new(WakuA2ANode::new(
        "alice",
        "Alice agent",
        vec!["text".into()],
        transport.clone(),
    ));
    let bob = Arc::new(WakuA2ANode::new(
        "bob",
        "Bob agent",
        vec!["text".into()],
        transport,
    ));
    (alice, bob)
}

#[tokio::test]
async fn test_send_and_receive_task() {
    let (alice, bob) = make_arc_pair();

    // Bob subscribes first (lazy init via poll_tasks)
    let empty = bob.poll_tasks().await.unwrap();
    assert!(empty.is_empty());

    // Alice sends task to Bob; Bob polls concurrently to ACK
    let bob_pubkey = bob.pubkey().to_string();
    let alice_clone = alice.clone();
    let send_handle =
        tokio::spawn(async move { alice_clone.send_text(&bob_pubkey, "ping").await.unwrap() });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let tasks = bob.poll_tasks().await.unwrap();
    let task = send_handle.await.unwrap();

    assert_eq!(task.text(), Some("ping"));
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].text(), Some("ping"));
    assert_eq!(tasks[0].from, alice.pubkey());
    assert_eq!(tasks[0].state, TaskState::Submitted);
}

#[tokio::test]
async fn test_full_request_response_cycle() {
    let (alice, bob) = make_arc_pair();

    // Both subscribe
    alice.poll_tasks().await.unwrap();
    bob.poll_tasks().await.unwrap();

    // Alice sends to Bob concurrently with Bob polling
    let bob_pubkey = bob.pubkey().to_string();
    let alice_clone = alice.clone();
    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_text(&bob_pubkey, "What is 2+2?")
            .await
            .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let tasks = bob.poll_tasks().await.unwrap();
    let task = send_handle.await.unwrap();

    assert_eq!(tasks.len(), 1);
    bob.respond(&tasks[0], "4").await.unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;

    // Alice receives response
    let responses = alice.poll_tasks().await.unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].state, TaskState::Completed);
    assert_eq!(responses[0].result_text(), Some("4"));
    assert_eq!(responses[0].id, task.id);
}

#[tokio::test]
async fn test_multiple_tasks() {
    let (alice, bob) = make_arc_pair();
    bob.poll_tasks().await.unwrap();

    // Send 3 tasks concurrently with Bob polling
    let bob_pubkey = bob.pubkey().to_string();
    let alice_clone = alice.clone();
    let bob_clone = bob.clone();

    let send_handle = tokio::spawn(async move {
        for i in 0..3 {
            alice_clone
                .send_text(&bob_pubkey, &format!("task-{}", i))
                .await
                .unwrap();
        }
    });

    // Poll multiple times to catch all messages and send ACKs
    let poll_handle = tokio::spawn(async move {
        let mut all_tasks = Vec::new();
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let tasks = bob_clone.poll_tasks().await.unwrap();
            all_tasks.extend(tasks);
            if all_tasks.len() >= 3 {
                break;
            }
        }
        all_tasks
    });

    send_handle.await.unwrap();
    let tasks = poll_handle.await.unwrap();
    assert_eq!(tasks.len(), 3);
}

#[tokio::test]
async fn test_discover_agents() {
    let transport = InMemoryTransport::new();
    let alice = WakuA2ANode::new(
        "alice",
        "Alice agent",
        vec!["text".into()],
        transport.clone(),
    );
    let bob = WakuA2ANode::new("bob", "Bob agent", vec!["code".into()], transport);

    alice.announce().await.unwrap();
    bob.announce().await.unwrap();

    // Alice discovers Bob (not herself)
    let cards = alice.discover().await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].name, "bob");
    assert_eq!(cards[0].capabilities, vec!["code"]);
}

#[tokio::test]
async fn test_encrypted_task_roundtrip() {
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

    // Alice sends encrypted task to Bob; Bob polls concurrently to send ACK
    let task = Task::new(alice.pubkey(), bob.pubkey(), "secret message");
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
    let tasks = bob.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].text(), Some("secret message"));

    // Bob responds encrypted; Alice polls concurrently
    let alice_card = alice.card.clone();
    let bob_clone = bob.clone();
    let task_ref = tasks[0].clone();

    let respond_handle = tokio::spawn(async move {
        bob_clone
            .respond_to(&task_ref, "acknowledged", Some(&alice_card))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    respond_handle.await.unwrap();

    // Alice receives encrypted response
    let responses = alice.poll_tasks().await.unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result_text(), Some("acknowledged"));
}

#[tokio::test]
async fn test_unencrypted_node_receives_plaintext_only() {
    let transport = InMemoryTransport::new();
    let alice = Arc::new(WakuA2ANode::new_encrypted(
        "alice",
        "Alice",
        vec![],
        transport.clone(),
    ));
    let bob = Arc::new(WakuA2ANode::new(
        "bob",
        "Bob (no encryption)",
        vec![],
        transport,
    ));

    bob.poll_tasks().await.unwrap();

    // Alice sends without encryption (no recipient card → falls back to plaintext)
    let bob_pubkey = bob.pubkey().to_string();
    let alice_clone = alice.clone();
    let bob_clone = bob.clone();

    let send_handle = tokio::spawn(async move {
        alice_clone
            .send_text(&bob_pubkey, "plain hello")
            .await
            .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let tasks = bob_clone.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].text(), Some("plain hello"));
}
