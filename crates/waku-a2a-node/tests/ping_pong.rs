//! Integration test: two nodes communicating via InMemoryTransport.

use std::time::Duration;
use waku_a2a_core::{topics, A2AEnvelope, Task};
use waku_a2a_node::WakuA2ANode;
use waku_a2a_transport::memory::InMemoryTransport;
use waku_a2a_transport::Transport;

#[tokio::test]
async fn test_discover_agents() {
    let transport = InMemoryTransport::new();

    let node_a = WakuA2ANode::new("agent-a", "Agent A", vec!["text".into()], transport.clone());
    let node_b = WakuA2ANode::new("agent-b", "Agent B", vec!["text".into()], transport.clone());

    // Node A announces
    node_a.announce().await.unwrap();

    // Node B discovers Node A
    let agents = node_b.discover().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "agent-a");
    assert_eq!(agents[0].public_key, node_a.pubkey());
}

#[tokio::test]
async fn test_ping_pong_plaintext() {
    let transport = InMemoryTransport::new();

    let node_a = WakuA2ANode::new("agent-a", "Agent A", vec!["text".into()], transport.clone());
    let node_b = WakuA2ANode::new("agent-b", "Agent B", vec!["text".into()], transport.clone());

    // Announce both
    node_a.announce().await.unwrap();
    node_b.announce().await.unwrap();

    // Node B discovers Node A
    let agents = node_b.discover().await.unwrap();
    assert!(agents.iter().any(|a| a.name == "agent-a"));

    // Node B sends a task to Node A (directly, bypassing SDS ACK wait)
    let task = Task::new(node_b.pubkey(), node_a.pubkey(), "Hello from B!");
    let envelope = A2AEnvelope::Task(task.clone());
    let payload = serde_json::to_vec(&envelope).unwrap();
    let topic = topics::task_topic(node_a.pubkey());

    // Ensure Node A is subscribed before publishing
    node_a.poll_tasks().await.unwrap();
    transport.publish(&topic, &payload).await.unwrap();

    // Node A receives the task
    let tasks = node_a.poll_tasks().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].text(), Some("Hello from B!"));
    assert_eq!(tasks[0].from, node_b.pubkey());

    // Node A replies
    node_a
        .respond(&tasks[0], "Hello back from A!")
        .await
        .unwrap();

    // Node B receives the response
    let responses = node_b.poll_tasks().await.unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result_text(), Some("Hello back from A!"));
}

#[tokio::test]
async fn test_ping_pong_with_sds() {
    let transport = InMemoryTransport::new();

    let node_a = WakuA2ANode::new("agent-a", "Agent A", vec!["text".into()], transport.clone());
    let node_b = WakuA2ANode::new("agent-b", "Agent B", vec!["text".into()], transport.clone());

    // Ensure Node A is subscribed to its task topic before Node B sends
    node_a.poll_tasks().await.unwrap();

    let task = Task::new(node_b.pubkey(), node_a.pubkey(), "Ping with SDS!");

    // Run send and receive concurrently — send_task waits for ACK,
    // poll_tasks sends ACK on receipt
    let (send_result, _) = tokio::join!(node_b.send_task(&task), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let tasks = node_a.poll_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text(), Some("Ping with SDS!"));
        node_a.respond(&tasks[0], "Pong!").await.unwrap();
    });

    // send_task should have received the ACK
    assert!(send_result.unwrap());
}

#[tokio::test]
async fn test_round_trip_within_timeout() {
    let start = std::time::Instant::now();

    let transport = InMemoryTransport::new();

    let node_a = WakuA2ANode::new("agent-a", "Agent A", vec!["text".into()], transport.clone());
    let node_b = WakuA2ANode::new("agent-b", "Agent B", vec!["text".into()], transport.clone());

    node_a.announce().await.unwrap();
    let agents = node_b.discover().await.unwrap();
    assert!(!agents.is_empty());

    // Direct task exchange (no SDS wait)
    let task = Task::new(node_b.pubkey(), node_a.pubkey(), "Speed test");
    let envelope = A2AEnvelope::Task(task.clone());
    let payload = serde_json::to_vec(&envelope).unwrap();
    let topic = topics::task_topic(node_a.pubkey());

    node_a.poll_tasks().await.unwrap();
    transport.publish(&topic, &payload).await.unwrap();

    let tasks = node_a.poll_tasks().await.unwrap();
    assert_eq!(tasks.len(), 1);

    node_a.respond(&tasks[0], "Fast!").await.unwrap();

    let responses = node_b.poll_tasks().await.unwrap();
    assert_eq!(responses.len(), 1);

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "Round-trip took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_encrypted_ping_pong() {
    let transport = InMemoryTransport::new();

    let node_a = WakuA2ANode::new_encrypted(
        "agent-a",
        "Agent A (encrypted)",
        vec!["text".into()],
        transport.clone(),
    );
    let node_b = WakuA2ANode::new_encrypted(
        "agent-b",
        "Agent B (encrypted)",
        vec!["text".into()],
        transport.clone(),
    );

    // Announce and discover
    node_a.announce().await.unwrap();
    node_b.announce().await.unwrap();

    let agents = node_b.discover().await.unwrap();
    let agent_a_card = agents.iter().find(|a| a.name == "agent-a").unwrap();
    assert!(agent_a_card.intro_bundle.is_some());

    // Node B sends encrypted task to Node A
    let task = Task::new(node_b.pubkey(), node_a.pubkey(), "Secret message!");
    let identity = node_b.identity().unwrap();
    let their_pubkey = waku_a2a_crypto::AgentIdentity::parse_public_key(
        &agent_a_card.intro_bundle.as_ref().unwrap().agent_pubkey,
    )
    .unwrap();
    let session_key = identity.shared_key(&their_pubkey);
    let task_json = serde_json::to_vec(&task).unwrap();
    let encrypted = session_key.encrypt(&task_json).unwrap();
    let envelope = A2AEnvelope::EncryptedTask {
        encrypted,
        sender_pubkey: identity.public_key_hex(),
    };
    let payload = serde_json::to_vec(&envelope).unwrap();
    let topic = topics::task_topic(node_a.pubkey());

    node_a.poll_tasks().await.unwrap();
    transport.publish(&topic, &payload).await.unwrap();

    // Node A decrypts and receives
    let tasks = node_a.poll_tasks().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].text(), Some("Secret message!"));

    // Node A responds encrypted
    node_a
        .respond_to(&tasks[0], "Secret reply!", Some(&node_b.card))
        .await
        .unwrap();

    // Node B receives encrypted response
    let responses = node_b.poll_tasks().await.unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result_text(), Some("Secret reply!"));
}
