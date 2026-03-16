//! Multi-agent task delegation: decompose tasks into subtasks and forward
//! them to capable peers discovered via presence.

use logos_messaging_a2a_core::{
    topics, A2AEnvelope, DelegationRequest, DelegationResult, DelegationStrategy, Task,
};
use logos_messaging_a2a_transport::Transport;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::{NodeError, Result, WakuA2ANode};

/// Default timeout for delegation when none is specified (30 seconds).
const DEFAULT_DELEGATION_TIMEOUT_SECS: u64 = 30;

impl<T: Transport> WakuA2ANode<T> {
    /// Delegate a subtask to a single peer based on the delegation strategy.
    ///
    /// Looks up peers from the live peer map, picks one according to the
    /// strategy, sends the subtask, and waits for a response within the
    /// specified timeout.
    pub async fn delegate_task(&self, request: &DelegationRequest) -> Result<DelegationResult> {
        let timeout_secs = if request.timeout_secs == 0 {
            DEFAULT_DELEGATION_TIMEOUT_SECS
        } else {
            request.timeout_secs
        };

        // Find a suitable peer based on strategy
        let peer_id = match &request.strategy {
            DelegationStrategy::FirstAvailable => {
                let peers = self.peers().all_live();
                peers.into_iter().map(|(id, _)| id).next().ok_or_else(|| {
                    NodeError::Other("no live peers available for delegation".into())
                })?
            }
            DelegationStrategy::CapabilityMatch { capability } => {
                let peers = self.find_peers_by_capability(capability);
                peers.into_iter().map(|(id, _)| id).next().ok_or_else(|| {
                    NodeError::Other(format!(
                        "no live peers with capability '{capability}' for delegation"
                    ))
                })?
            }
            DelegationStrategy::BroadcastCollect => {
                // For single delegation, broadcast acts like first-available
                let peers = self.peers().all_live();
                peers.into_iter().map(|(id, _)| id).next().ok_or_else(|| {
                    NodeError::Other("no live peers available for broadcast delegation".into())
                })?
            }
            DelegationStrategy::RoundRobin => {
                let peers: Vec<String> = self
                    .peers()
                    .all_live()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                if peers.is_empty() {
                    return Err(NodeError::Other(
                        "no live peers available for round-robin delegation".into(),
                    ));
                }
                let idx = self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % peers.len();
                peers.into_iter().nth(idx).unwrap()
            }
        };

        self.delegate_to_peer(request, &peer_id, timeout_secs).await
    }

    /// Delegate a subtask to all matching peers and collect responses.
    ///
    /// Distributes the subtask to every peer that matches the delegation
    /// strategy and waits up to `timeout_secs` for responses from all of them.
    pub async fn delegate_broadcast(
        &self,
        request: &DelegationRequest,
    ) -> Result<Vec<DelegationResult>> {
        let timeout_secs = if request.timeout_secs == 0 {
            DEFAULT_DELEGATION_TIMEOUT_SECS
        } else {
            request.timeout_secs
        };

        let peer_ids: Vec<String> = match &request.strategy {
            DelegationStrategy::CapabilityMatch { capability } => self
                .find_peers_by_capability(capability)
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
            // RoundRobin, BroadcastCollect, FirstAvailable all broadcast to every peer
            _ => self
                .peers()
                .all_live()
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
        };

        if peer_ids.is_empty() {
            return Err(NodeError::Other(
                "no live peers available for broadcast delegation".into(),
            ));
        }

        let mut results = Vec::new();
        for peer_id in peer_ids {
            let result = self.delegate_to_peer(request, &peer_id, timeout_secs).await;
            match result {
                Ok(r) => results.push(r),
                Err(e) => results.push(DelegationResult {
                    parent_task_id: request.parent_task_id.clone(),
                    subtask_id: String::new(),
                    agent_id: peer_id,
                    result_text: None,
                    success: false,
                    error: Some(e.to_string()),
                }),
            }
        }

        Ok(results)
    }

    /// Send a subtask to a specific peer and wait for a response.
    ///
    /// This is an internal helper used by both [`delegate_task`] and
    /// [`delegate_broadcast`].
    ///
    /// Uses fire-and-forget send (like `respond`) rather than SDS reliable
    /// delivery, since delegation already has its own response-based timeout.
    async fn delegate_to_peer(
        &self,
        request: &DelegationRequest,
        peer_id: &str,
        timeout_secs: u64,
    ) -> Result<DelegationResult> {
        let task = Task::new(self.pubkey(), peer_id, &request.subtask_text);
        let subtask_id = task.id.clone();

        // Publish directly to transport (bypassing SDS reliable delivery)
        // since delegation already polls for the response with its own timeout.
        let topic = topics::task_topic(peer_id);
        let envelope = A2AEnvelope::Task(task);
        let payload = serde_json::to_vec(&envelope)?;
        self.channel().transport().publish(&topic, &payload).await?;

        // Poll for response with timeout
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

        while tokio::time::Instant::now() < deadline {
            let tasks = self.poll_tasks().await?;
            for received in &tasks {
                if received.id == subtask_id {
                    return Ok(DelegationResult {
                        parent_task_id: request.parent_task_id.clone(),
                        subtask_id: subtask_id.clone(),
                        agent_id: peer_id.to_string(),
                        result_text: received.result_text().map(String::from),
                        success: true,
                        error: None,
                    });
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        Ok(DelegationResult {
            parent_task_id: request.parent_task_id.clone(),
            subtask_id,
            agent_id: peer_id.to_string(),
            result_text: None,
            success: false,
            error: Some("delegation timed out".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::{DelegationRequest, DelegationStrategy};
    use logos_messaging_a2a_transport::memory::InMemoryTransport;
    use logos_messaging_a2a_transport::sds::ChannelConfig;
    use std::time::Duration;

    /// Fast SDS config that skips ACK waits — suitable for in-process tests.
    fn fast_config() -> ChannelConfig {
        ChannelConfig {
            ack_timeout: Duration::from_millis(1),
            max_retries: 0,
            ..Default::default()
        }
    }

    /// Create a node with fast SDS config, announce presence, and subscribe
    /// to its own task topic (lazy-init).
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
        node.poll_tasks().await.unwrap();
        node
    }

    /// Poll for one incoming task and respond with an echo.
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

    /// Poll for exactly `n` incoming tasks and respond to each.
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

    // ── 1. FirstAvailable — success ──

    #[tokio::test]
    async fn delegate_task_first_available_success() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        let worker = make_announced_node("worker", vec!["text"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-001".into(),
            subtask_text: "Hello worker".into(),
            strategy: DelegationStrategy::FirstAvailable,
            timeout_secs: 5,
        };

        let wh = tokio::spawn(async move { echo_once(&worker).await });
        let result = orchestrator.delegate_task(&request).await.unwrap();
        wh.await.unwrap();

        assert!(result.success);
        assert_eq!(result.parent_task_id, "p-001");
        assert!(result.result_text.unwrap().contains("echo: Hello worker"));
        assert!(result.error.is_none());
    }

    // ── 2. CapabilityMatch — correct peer selected ──

    #[tokio::test]
    async fn delegate_task_capability_match_finds_correct_peer() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        let summarizer =
            make_announced_node("summarizer", vec!["summarize"], transport.clone()).await;
        let _coder = make_announced_node("coder", vec!["code"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let summarizer_pubkey = summarizer.pubkey().to_string();

        let request = DelegationRequest {
            parent_task_id: "p-002".into(),
            subtask_text: "Summarize this".into(),
            strategy: DelegationStrategy::CapabilityMatch {
                capability: "summarize".into(),
            },
            timeout_secs: 5,
        };

        let sh = tokio::spawn(async move { echo_once(&summarizer).await });
        let result = orchestrator.delegate_task(&request).await.unwrap();
        sh.await.unwrap();

        assert!(result.success);
        assert_eq!(result.agent_id, summarizer_pubkey);
        assert!(result.result_text.unwrap().contains("echo: Summarize this"));
    }

    // ── 3. CapabilityMatch — no matching peers ──

    #[tokio::test]
    async fn delegate_task_capability_match_no_matching_peers() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        let _worker = make_announced_node("worker", vec!["text"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-003".into(),
            subtask_text: "Need image processing".into(),
            strategy: DelegationStrategy::CapabilityMatch {
                capability: "image".into(),
            },
            timeout_secs: 1,
        };

        let err = orchestrator.delegate_task(&request).await.unwrap_err();
        assert!(
            err.to_string().contains("no live peers with capability"),
            "unexpected error: {err}"
        );
    }

    // ── 4. No live peers at all ──

    #[tokio::test]
    async fn delegate_task_no_live_peers_error() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;

        // No other nodes — peer map is empty
        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-004".into(),
            subtask_text: "Nobody home".into(),
            strategy: DelegationStrategy::FirstAvailable,
            timeout_secs: 1,
        };

        let err = orchestrator.delegate_task(&request).await.unwrap_err();
        assert!(
            err.to_string().contains("no live peers"),
            "unexpected error: {err}"
        );
    }

    // ── 5. RoundRobin — distributes across peers ──

    #[tokio::test]
    async fn delegate_task_round_robin_distributes() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        let worker_a = make_announced_node("worker-a", vec!["text"], transport.clone()).await;
        let worker_b = make_announced_node("worker-b", vec!["text"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let peer_ids: Vec<String> = orchestrator
            .peers()
            .all_live()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(peer_ids.len(), 2);

        // Each worker handles 2 tasks (4 total / 2 workers)
        let ha = tokio::spawn(async move { echo_n(&worker_a, 2).await });
        let hb = tokio::spawn(async move { echo_n(&worker_b, 2).await });

        let mut agent_ids = Vec::new();
        for i in 0..4 {
            let request = DelegationRequest {
                parent_task_id: format!("rr-{i}"),
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

    // ── 6. Timeout — peer does not respond ──

    #[tokio::test]
    async fn delegate_task_timeout_returns_failure() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        // Silent worker announces but never responds to tasks
        let _silent = make_announced_node("silent", vec!["text"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-006".into(),
            subtask_text: "No reply expected".into(),
            strategy: DelegationStrategy::FirstAvailable,
            timeout_secs: 1,
        };

        let result = orchestrator.delegate_task(&request).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("delegation timed out"));
        assert!(result.result_text.is_none());
    }

    // ── 7. Broadcast — collects multiple responses ──

    #[tokio::test]
    async fn delegate_broadcast_collects_results() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        let worker_a = make_announced_node("worker-a", vec!["text"], transport.clone()).await;
        let worker_b = make_announced_node("worker-b", vec!["text"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-007".into(),
            subtask_text: "Broadcast task".into(),
            strategy: DelegationStrategy::BroadcastCollect,
            timeout_secs: 10,
        };

        let ha = tokio::spawn(async move { echo_once(&worker_a).await });
        let hb = tokio::spawn(async move { echo_once(&worker_b).await });

        let results = orchestrator.delegate_broadcast(&request).await.unwrap();
        ha.await.unwrap();
        hb.await.unwrap();

        assert_eq!(results.len(), 2);
        let successes: Vec<_> = results.iter().filter(|r| r.success).collect();
        assert!(
            !successes.is_empty(),
            "at least one broadcast result should succeed"
        );
        for r in &successes {
            assert_eq!(r.parent_task_id, "p-007");
            assert!(r
                .result_text
                .as_ref()
                .unwrap()
                .contains("echo: Broadcast task"));
        }
    }

    // ── 8. Broadcast — no peers ──

    #[tokio::test]
    async fn delegate_broadcast_no_peers_error() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-008".into(),
            subtask_text: "Nobody".into(),
            strategy: DelegationStrategy::BroadcastCollect,
            timeout_secs: 1,
        };

        let err = orchestrator.delegate_broadcast(&request).await.unwrap_err();
        assert!(
            err.to_string().contains("no live peers"),
            "unexpected error: {err}"
        );
    }

    // ── 9. Broadcast — mixed success/failure ──

    #[tokio::test]
    async fn delegate_broadcast_mixed_success_failure() {
        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        let worker = make_announced_node("worker", vec!["text"], transport.clone()).await;
        // silent announces but never responds
        let _silent = make_announced_node("silent", vec!["text"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-009".into(),
            subtask_text: "Partial broadcast".into(),
            strategy: DelegationStrategy::CapabilityMatch {
                capability: "text".into(),
            },
            timeout_secs: 2,
        };

        let wh = tokio::spawn(async move { echo_once(&worker).await });
        let results = orchestrator.delegate_broadcast(&request).await.unwrap();
        wh.await.unwrap();

        assert_eq!(results.len(), 2);
        let successes = results.iter().filter(|r| r.success).count();
        let failures = results.iter().filter(|r| !r.success).count();
        assert_eq!(successes, 1);
        assert_eq!(failures, 1);

        // The failing result should carry a timeout error
        let fail = results.iter().find(|r| !r.success).unwrap();
        assert_eq!(fail.error.as_deref(), Some("delegation timed out"));
    }

    // ── 10. Default timeout when request.timeout_secs == 0 ──

    #[tokio::test]
    async fn default_timeout_used_when_request_timeout_is_zero() {
        // Verify the constant is 30 seconds.
        assert_eq!(super::DEFAULT_DELEGATION_TIMEOUT_SECS, 30);

        let transport = InMemoryTransport::new();
        let orchestrator =
            make_announced_node("orchestrator", vec!["coordinate"], transport.clone()).await;
        let worker = make_announced_node("worker", vec!["text"], transport.clone()).await;

        orchestrator.poll_presence().await.unwrap();

        let request = DelegationRequest {
            parent_task_id: "p-010".into(),
            subtask_text: "Quick task".into(),
            strategy: DelegationStrategy::FirstAvailable,
            timeout_secs: 0, // should fall back to DEFAULT_DELEGATION_TIMEOUT_SECS
        };

        let wh = tokio::spawn(async move { echo_once(&worker).await });
        let result = orchestrator.delegate_task(&request).await.unwrap();
        wh.await.unwrap();

        // If the default were 0, this would have timed out immediately.
        assert!(result.success);
    }
}
