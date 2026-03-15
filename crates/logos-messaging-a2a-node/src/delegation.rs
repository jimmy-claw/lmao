//! Multi-agent task delegation: decompose tasks into subtasks and forward
//! them to capable peers discovered via presence.

use anyhow::{Context, Result};
use logos_messaging_a2a_core::{
    topics, A2AEnvelope, DelegationRequest, DelegationResult, DelegationStrategy, Task,
};
use logos_messaging_a2a_transport::Transport;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::WakuA2ANode;

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
                peers
                    .into_iter()
                    .map(|(id, _)| id)
                    .next()
                    .context("no live peers available for delegation")?
            }
            DelegationStrategy::CapabilityMatch { capability } => {
                let peers = self.find_peers_by_capability(capability);
                peers
                    .into_iter()
                    .map(|(id, _)| id)
                    .next()
                    .with_context(|| {
                        format!("no live peers with capability '{capability}' for delegation")
                    })?
            }
            DelegationStrategy::BroadcastCollect => {
                // For single delegation, broadcast acts like first-available
                let peers = self.peers().all_live();
                peers
                    .into_iter()
                    .map(|(id, _)| id)
                    .next()
                    .context("no live peers available for broadcast delegation")?
            }
            DelegationStrategy::RoundRobin => {
                let peers: Vec<String> = self
                    .peers()
                    .all_live()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                if peers.is_empty() {
                    anyhow::bail!("no live peers available for round-robin delegation");
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
            anyhow::bail!("no live peers available for broadcast delegation");
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
        let payload =
            serde_json::to_vec(&envelope).context("failed to serialize delegated subtask")?;
        self.channel()
            .transport()
            .publish(&topic, &payload)
            .await
            .context("failed to send delegated subtask")?;

        // Poll for response with timeout
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

        while tokio::time::Instant::now() < deadline {
            let tasks = self
                .poll_tasks()
                .await
                .context("failed to poll for delegation response")?;
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
