//! Integration tests for multi-agent task delegation.

use logos_messaging_a2a_core::{DelegationRequest, DelegationStrategy};
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::memory::InMemoryTransport;
use logos_messaging_a2a_transport::sds::ChannelConfig;
use std::time::Duration;

/// Fast SDS config that doesn't block on ACK, suitable for tests.
fn fast_config() -> ChannelConfig {
    ChannelConfig {
        ack_timeout: Duration::from_millis(1),
        max_retries: 0,
        ..Default::default()
    }
}

/// Helper: create a node with fast SDS config and announce its presence.
async fn make_announced_node(
    name: &str,
    capabilities: Vec<&str>,
    transport: InMemoryTransport,
) -> WakuA2ANode<InMemoryTransport> {
    let caps = capabilities.into_iter().map(String::from).collect();
    let node = WakuA2ANode::with_config(
        name,
        &format!("{name} agent"),
        caps,
        transport,
        fast_config(),
    );
    node.announce_presence().await.unwrap();
    // Subscribe to own task topic (lazy init)
    node.poll_tasks().await.unwrap();
    node
}

/// Helper: run a simple echo-agent loop that responds to one task.
async fn echo_once(node: &WakuA2ANode<InMemoryTransport>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        let tasks = node.poll_tasks().await.unwrap();
        for task in &tasks {
            if task.result.is_none() {
                let reply = format!("echo: {}", task.text().unwrap_or(""));
                node.respond(task, &reply).await.unwrap();
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("echo_once timed out waiting for a task");
}

#[tokio::test]
async fn delegate_task_to_first_available_peer() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker = make_announced_node("worker", vec!["text"], transport.clone()).await;

    // Orchestrator polls presence to discover worker
    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-001".to_string(),
        subtask_text: "Hello worker".to_string(),
        strategy: DelegationStrategy::FirstAvailable,
        timeout_secs: 5,
    };

    // Worker echoes in background
    let worker_handle = tokio::spawn(async move {
        echo_once(&worker).await;
    });

    let result = orchestrator.delegate_task(&request).await.unwrap();
    worker_handle.await.unwrap();

    assert!(result.success);
    assert_eq!(result.parent_task_id, "parent-001");
    assert!(result.result_text.is_some());
    assert!(result.result_text.unwrap().contains("echo: Hello worker"));
    assert!(result.error.is_none());
}

#[tokio::test]
async fn delegate_task_with_capability_match() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let summarizer = make_announced_node("summarizer", vec!["summarize"], transport.clone()).await;
    let _coder = make_announced_node("coder", vec!["code"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-002".to_string(),
        subtask_text: "Summarize this".to_string(),
        strategy: DelegationStrategy::CapabilityMatch {
            capability: "summarize".to_string(),
        },
        timeout_secs: 5,
    };

    let summarizer_handle = tokio::spawn(async move {
        echo_once(&summarizer).await;
    });

    let result = orchestrator.delegate_task(&request).await.unwrap();
    summarizer_handle.await.unwrap();

    assert!(result.success);
    assert_eq!(result.parent_task_id, "parent-002");
    assert!(result.result_text.unwrap().contains("echo: Summarize this"));
}

#[tokio::test]
async fn delegate_task_no_peers_fails() {
    let transport = InMemoryTransport::new();
    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;

    // Don't create any workers — no peers in the map
    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-003".to_string(),
        subtask_text: "Nobody home".to_string(),
        strategy: DelegationStrategy::FirstAvailable,
        timeout_secs: 1,
    };

    let err = orchestrator.delegate_task(&request).await.unwrap_err();
    assert!(err.to_string().contains("no live peers"));
}

#[tokio::test]
async fn delegate_task_no_matching_capability_fails() {
    let transport = InMemoryTransport::new();
    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let _worker = make_announced_node("worker", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-004".to_string(),
        subtask_text: "Need image processing".to_string(),
        strategy: DelegationStrategy::CapabilityMatch {
            capability: "image".to_string(),
        },
        timeout_secs: 1,
    };

    let err = orchestrator.delegate_task(&request).await.unwrap_err();
    assert!(err.to_string().contains("no live peers with capability"));
}

#[tokio::test]
async fn delegate_task_timeout_returns_failure_result() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    // Worker announces but never responds
    let _silent_worker = make_announced_node("silent", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-005".to_string(),
        subtask_text: "No reply expected".to_string(),
        strategy: DelegationStrategy::FirstAvailable,
        timeout_secs: 1,
    };

    let result = orchestrator.delegate_task(&request).await.unwrap();
    assert!(!result.success);
    assert_eq!(result.error, Some("delegation timed out".to_string()));
    assert!(result.result_text.is_none());
}

#[tokio::test]
async fn delegate_broadcast_collects_multiple_responses() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker_a = make_announced_node("worker-a", vec!["text"], transport.clone()).await;
    let worker_b = make_announced_node("worker-b", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-006".to_string(),
        subtask_text: "Broadcast task".to_string(),
        strategy: DelegationStrategy::CapabilityMatch {
            capability: "text".to_string(),
        },
        timeout_secs: 10,
    };

    // Both workers echo in background
    let ha = tokio::spawn(async move { echo_once(&worker_a).await });
    let hb = tokio::spawn(async move { echo_once(&worker_b).await });

    let results = orchestrator.delegate_broadcast(&request).await.unwrap();
    ha.await.unwrap();
    hb.await.unwrap();

    assert_eq!(results.len(), 2);
    // At least one should succeed (both workers are echoing)
    let successes: Vec<_> = results.iter().filter(|r| r.success).collect();
    assert!(
        !successes.is_empty(),
        "at least one broadcast delegate should succeed"
    );
    for r in &successes {
        assert_eq!(r.parent_task_id, "parent-006");
        assert!(r
            .result_text
            .as_ref()
            .unwrap()
            .contains("echo: Broadcast task"));
    }
}

#[tokio::test]
async fn delegate_broadcast_no_peers_fails() {
    let transport = InMemoryTransport::new();
    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-007".to_string(),
        subtask_text: "Nobody".to_string(),
        strategy: DelegationStrategy::BroadcastCollect,
        timeout_secs: 1,
    };

    let err = orchestrator.delegate_broadcast(&request).await.unwrap_err();
    assert!(err.to_string().contains("no live peers"));
}

#[tokio::test]
async fn delegate_task_default_timeout_when_zero() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker = make_announced_node("worker", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-008".to_string(),
        subtask_text: "Quick task".to_string(),
        strategy: DelegationStrategy::FirstAvailable,
        timeout_secs: 0, // should use default
    };

    let worker_handle = tokio::spawn(async move {
        echo_once(&worker).await;
    });

    let result = orchestrator.delegate_task(&request).await.unwrap();
    worker_handle.await.unwrap();

    assert!(result.success);
}

#[tokio::test]
async fn delegate_broadcast_partial_timeout() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker = make_announced_node("worker", vec!["text"], transport.clone()).await;
    // silent_worker announces but never responds
    let _silent = make_announced_node("silent", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-009".to_string(),
        subtask_text: "Partial broadcast".to_string(),
        strategy: DelegationStrategy::CapabilityMatch {
            capability: "text".to_string(),
        },
        timeout_secs: 2,
    };

    let worker_handle = tokio::spawn(async move {
        echo_once(&worker).await;
    });

    let results = orchestrator.delegate_broadcast(&request).await.unwrap();
    worker_handle.await.unwrap();

    assert_eq!(results.len(), 2);
    let successes = results.iter().filter(|r| r.success).count();
    let failures = results.iter().filter(|r| !r.success).count();
    // One should succeed (worker), one should fail (silent)
    assert_eq!(successes, 1);
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn delegation_result_carries_correct_subtask_id() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker = make_announced_node("worker", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "parent-010".to_string(),
        subtask_text: "Check subtask ID".to_string(),
        strategy: DelegationStrategy::FirstAvailable,
        timeout_secs: 5,
    };

    let worker_handle = tokio::spawn(async move {
        echo_once(&worker).await;
    });

    let result = orchestrator.delegate_task(&request).await.unwrap();
    worker_handle.await.unwrap();

    assert!(result.success);
    // subtask_id should be a valid UUID (not empty)
    assert!(!result.subtask_id.is_empty());
    assert_ne!(result.subtask_id, result.parent_task_id);
}

// ── Round-Robin Delegation Tests ──

/// Helper: run an echo-agent loop that responds to N tasks.
async fn echo_n(node: &WakuA2ANode<InMemoryTransport>, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut count = 0;
    while count < n && tokio::time::Instant::now() < deadline {
        let tasks = node.poll_tasks().await.unwrap();
        for task in &tasks {
            if task.result.is_none() {
                let reply = format!(
                    "echo from {}: {}",
                    node.card.name,
                    task.text().unwrap_or("")
                );
                node.respond(task, &reply).await.unwrap();
                count += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(count, n, "echo_n: expected {n} tasks but got {count}");
}

#[tokio::test]
async fn round_robin_distributes_evenly() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker_a = make_announced_node("worker-a", vec!["text"], transport.clone()).await;
    let worker_b = make_announced_node("worker-b", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let peers = orchestrator.peers().all_live();
    assert_eq!(peers.len(), 2, "should see exactly 2 workers");

    // Collect peer IDs in the order the peer map returns them
    let peer_ids: Vec<String> = peers.into_iter().map(|(id, _)| id).collect();

    // Spawn workers: each expects 2 tasks (4 total / 2 workers)
    let wa = worker_a;
    let wb = worker_b;
    let ha = tokio::spawn(async move { echo_n(&wa, 2).await });
    let hb = tokio::spawn(async move { echo_n(&wb, 2).await });

    // Send 4 round-robin delegations
    let mut agent_ids = Vec::new();
    for i in 0..4 {
        let request = DelegationRequest {
            parent_task_id: format!("rr-parent-{i}"),
            subtask_text: format!("round-robin task {i}"),
            strategy: DelegationStrategy::RoundRobin,
            timeout_secs: 5,
        };
        let result = orchestrator.delegate_task(&request).await.unwrap();
        assert!(result.success, "task {i} should succeed");
        agent_ids.push(result.agent_id);
    }

    ha.await.unwrap();
    hb.await.unwrap();

    // Should alternate: peer0, peer1, peer0, peer1
    assert_eq!(agent_ids[0], peer_ids[0]);
    assert_eq!(agent_ids[1], peer_ids[1]);
    assert_eq!(agent_ids[2], peer_ids[0]);
    assert_eq!(agent_ids[3], peer_ids[1]);
}

#[tokio::test]
async fn round_robin_wraps_around() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker_a = make_announced_node("worker-a", vec!["text"], transport.clone()).await;
    let worker_b = make_announced_node("worker-b", vec!["text"], transport.clone()).await;
    let worker_c = make_announced_node("worker-c", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let peers = orchestrator.peers().all_live();
    assert_eq!(peers.len(), 3, "should see exactly 3 workers");

    let peer_ids: Vec<String> = peers.into_iter().map(|(id, _)| id).collect();

    // Each worker expects 2 tasks (6 total / 3 workers)
    let ha = tokio::spawn(async move { echo_n(&worker_a, 2).await });
    let hb = tokio::spawn(async move { echo_n(&worker_b, 2).await });
    let hc = tokio::spawn(async move { echo_n(&worker_c, 2).await });

    // Send 6 round-robin delegations to wrap around twice
    let mut agent_ids = Vec::new();
    for i in 0..6 {
        let request = DelegationRequest {
            parent_task_id: format!("rr-wrap-{i}"),
            subtask_text: format!("wrap task {i}"),
            strategy: DelegationStrategy::RoundRobin,
            timeout_secs: 5,
        };
        let result = orchestrator.delegate_task(&request).await.unwrap();
        assert!(result.success, "task {i} should succeed");
        agent_ids.push(result.agent_id);
    }

    ha.await.unwrap();
    hb.await.unwrap();
    hc.await.unwrap();

    // Should cycle: 0, 1, 2, 0, 1, 2
    for i in 0..6 {
        assert_eq!(
            agent_ids[i],
            peer_ids[i % 3],
            "task {i} should go to peer {}",
            i % 3
        );
    }
}

#[tokio::test]
async fn round_robin_single_peer() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker = make_announced_node("solo-worker", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let worker_pubkey = worker.pubkey().to_string();

    // Worker expects 3 tasks
    let wh = tokio::spawn(async move { echo_n(&worker, 3).await });

    // All 3 tasks should go to the single peer
    for i in 0..3 {
        let request = DelegationRequest {
            parent_task_id: format!("rr-single-{i}"),
            subtask_text: format!("solo task {i}"),
            strategy: DelegationStrategy::RoundRobin,
            timeout_secs: 5,
        };
        let result = orchestrator.delegate_task(&request).await.unwrap();
        assert!(result.success, "task {i} should succeed");
        assert_eq!(
            result.agent_id, worker_pubkey,
            "all tasks should go to the single worker"
        );
    }

    wh.await.unwrap();
}

#[tokio::test]
async fn round_robin_no_peers_fails() {
    let transport = InMemoryTransport::new();
    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "rr-none".to_string(),
        subtask_text: "Nobody home".to_string(),
        strategy: DelegationStrategy::RoundRobin,
        timeout_secs: 1,
    };

    let err = orchestrator.delegate_task(&request).await.unwrap_err();
    assert!(err.to_string().contains("no live peers"));
}

#[tokio::test]
async fn round_robin_broadcast_sends_to_all() {
    let transport = InMemoryTransport::new();

    let orchestrator =
        make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
    let worker_a = make_announced_node("worker-a", vec!["text"], transport.clone()).await;
    let worker_b = make_announced_node("worker-b", vec!["text"], transport.clone()).await;

    orchestrator.poll_presence().await.unwrap();

    let request = DelegationRequest {
        parent_task_id: "rr-broadcast".to_string(),
        subtask_text: "Broadcast via round-robin".to_string(),
        strategy: DelegationStrategy::RoundRobin,
        timeout_secs: 5,
    };

    let ha = tokio::spawn(async move { echo_once(&worker_a).await });
    let hb = tokio::spawn(async move { echo_once(&worker_b).await });

    let results = orchestrator.delegate_broadcast(&request).await.unwrap();
    ha.await.unwrap();
    hb.await.unwrap();

    // Broadcast should send to all peers, like BroadcastCollect
    assert_eq!(results.len(), 2);
    let successes: Vec<_> = results.iter().filter(|r| r.success).collect();
    assert!(!successes.is_empty(), "at least one should succeed");
}
