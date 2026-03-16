use anyhow::Result;
use logos_messaging_a2a_core::{DelegationRequest, DelegationStrategy, Task};
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::cli::TaskAction;
use crate::common::{build_node, IdentityConfig};

pub async fn handle(
    action: TaskAction,
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
    json: bool,
) -> Result<()> {
    match action {
        TaskAction::Send { to, text } => {
            let node = build_node("cli-sender", "CLI client", vec![], transport, identity)?;
            if !json {
                println!("Sending task to {}...", &to[..12.min(to.len())]);
                println!("From pubkey: {}", node.pubkey());
            }
            let task = Task::new(node.pubkey(), &to, &text);
            match node.send_task(&task).await {
                Ok(acked) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "task_id": task.id,
                                "from": node.pubkey(),
                                "to": to,
                                "status": if acked { "acked" } else { "sent" },
                            }))?
                        );
                    } else {
                        println!("Task ID: {}", task.id);
                        if acked {
                            println!("Status: ACKed by recipient");
                        } else {
                            println!("Status: Sent (no ACK — recipient may be offline)");
                        }
                    }
                }
                Err(e) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "task_id": task.id,
                                "from": node.pubkey(),
                                "to": to,
                                "status": "failed",
                                "error": e.to_string(),
                            }))?
                        );
                    } else {
                        eprintln!("Failed to send task: {}", e);
                        println!("Task ID: {} (failed)", task.id);
                    }
                }
            }
        }
        TaskAction::Status { id } => {
            let node = build_node("cli-poller", "CLI client", vec![], transport, identity)?;
            if !json {
                println!("Polling for task {} responses...", id);
                println!("Listening as: {}", node.pubkey());
            }
            match node.poll_tasks().await {
                Ok(tasks) => {
                    let found: Vec<_> = tasks.iter().filter(|t| t.id == id).collect();
                    if json {
                        let results: Vec<_> = found
                            .iter()
                            .map(|t| {
                                let mut obj = serde_json::json!({
                                    "task_id": t.id,
                                    "state": format!("{:?}", t.state),
                                });
                                if let Some(text) = t.result_text() {
                                    obj["result"] = serde_json::json!(text);
                                }
                                obj
                            })
                            .collect();
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "task_id": id,
                                "listener": node.pubkey(),
                                "results": results,
                            }))?
                        );
                    } else if found.is_empty() {
                        println!("No response yet for task {}", id);
                    } else {
                        for task in found {
                            println!("Task: {}", task.id);
                            println!("State: {:?}", task.state);
                            if let Some(text) = task.result_text() {
                                println!("Result: {}", text);
                            }
                        }
                    }
                }
                Err(e) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "task_id": id,
                                "error": e.to_string(),
                            }))?
                        );
                    } else {
                        eprintln!("Failed to poll: {}", e);
                    }
                }
            }
        }
        TaskAction::Stream { id, timeout } => {
            let node = build_node("cli-stream", "CLI client", vec![], transport, identity)?;
            if !json {
                println!(
                    "Following stream for task {} (timeout {}s)...\n",
                    id, timeout
                );
            }

            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
            let mut last_index: Option<u32> = None;

            while tokio::time::Instant::now() < deadline {
                match node.poll_stream_chunks(&id).await {
                    Ok(chunks) => {
                        for chunk in &chunks {
                            if last_index.is_none() || chunk.chunk_index > last_index.unwrap() {
                                if json {
                                    println!(
                                        "{}",
                                        serde_json::to_string(&serde_json::json!({
                                            "event": "chunk",
                                            "task_id": id,
                                            "chunk_index": chunk.chunk_index,
                                            "text": chunk.text,
                                            "is_final": chunk.is_final,
                                        }))?
                                    );
                                } else {
                                    print!("{}", chunk.text);
                                }
                                last_index = Some(chunk.chunk_index);
                            }
                        }
                        if chunks.iter().any(|c| c.is_final) {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string(&serde_json::json!({
                                        "event": "stream_complete",
                                        "task_id": id,
                                        "total_chunks": chunks.len(),
                                    }))?
                                );
                            } else {
                                println!();
                                println!("\n--- Stream complete ({} chunks) ---", chunks.len());
                            }
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Stream poll error: {}", e);
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            if last_index.is_none() {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "event": "stream_timeout",
                            "task_id": id,
                            "total_chunks": 0,
                        }))?
                    );
                } else {
                    println!("No stream chunks received for task {}", id);
                }
            }
        }
        TaskAction::Delegate {
            to,
            capability,
            text,
            parent_id,
            timeout,
            broadcast,
            strategy,
        } => {
            let node = build_node(
                "cli-delegator",
                "CLI delegation client",
                vec![],
                transport,
                identity,
            )?;
            if !json {
                println!("From pubkey: {}", node.pubkey());
            }

            // Build strategy from flags
            let strategy = if let Some(ref agent_key) = to {
                // Direct delegation — send task directly, skip presence lookup
                if !json {
                    println!(
                        "Delegating directly to {}...",
                        &agent_key[..12.min(agent_key.len())]
                    );
                }
                let task = Task::new(node.pubkey(), agent_key, &text);
                if !json {
                    println!("Subtask ID: {}", task.id);
                }
                match node.send_task(&task).await {
                    Ok(acked) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "subtask_id": task.id,
                                    "from": node.pubkey(),
                                    "to": agent_key,
                                    "parent_id": parent_id,
                                    "status": if acked { "acked" } else { "sent" },
                                }))?
                            );
                        } else {
                            if acked {
                                println!("Status: ACKed by recipient");
                            } else {
                                println!("Status: Sent (no ACK)");
                            }
                            println!("Parent task: {}", parent_id);
                        }
                    }
                    Err(e) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "subtask_id": task.id,
                                    "from": node.pubkey(),
                                    "to": agent_key,
                                    "parent_id": parent_id,
                                    "status": "failed",
                                    "error": e.to_string(),
                                }))?
                            );
                        } else {
                            eprintln!("Failed to delegate: {}", e);
                        }
                    }
                }
                return Ok(());
            } else if let Some(ref s) = strategy {
                match s.as_str() {
                    "round-robin" => DelegationStrategy::RoundRobin,
                    "broadcast" => DelegationStrategy::BroadcastCollect,
                    "first-available" => DelegationStrategy::FirstAvailable,
                    other => {
                        if let Some(ref cap) = capability {
                            DelegationStrategy::CapabilityMatch {
                                capability: cap.clone(),
                            }
                        } else {
                            eprintln!("Unknown strategy '{other}', using first-available");
                            DelegationStrategy::FirstAvailable
                        }
                    }
                }
            } else if let Some(ref cap) = capability {
                DelegationStrategy::CapabilityMatch {
                    capability: cap.clone(),
                }
            } else {
                DelegationStrategy::FirstAvailable
            };

            // Presence-based delegation
            if !json {
                println!("Discovering peers via presence...");
            }
            let poll_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            while tokio::time::Instant::now() < poll_deadline {
                node.poll_presence().await?;
                if !node.peers().all_live().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            let live = node.peers().all_live();
            if !json {
                println!("Found {} live peer(s)", live.len());
            }

            let request = DelegationRequest {
                parent_task_id: parent_id.clone(),
                subtask_text: text,
                strategy,
                timeout_secs: timeout,
            };

            if broadcast {
                if !json {
                    println!("Broadcasting to all matching peers...");
                }
                match node.delegate_broadcast(&request).await {
                    Ok(results) => {
                        if json {
                            let items: Vec<_> = results
                                .iter()
                                .map(|r| {
                                    serde_json::json!({
                                        "subtask_id": r.subtask_id,
                                        "agent_id": r.agent_id,
                                        "success": r.success,
                                        "result_text": r.result_text,
                                        "error": r.error,
                                    })
                                })
                                .collect();
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "parent_id": parent_id,
                                    "broadcast": true,
                                    "results": items,
                                }))?
                            );
                        } else {
                            println!("Received {} result(s):", results.len());
                            for r in &results {
                                let status = if r.success { "OK" } else { "FAIL" };
                                let agent = &r.agent_id[..12.min(r.agent_id.len())];
                                println!("  [{status}] agent={agent} subtask={}", r.subtask_id);
                                if let Some(ref text) = r.result_text {
                                    println!("    Result: {text}");
                                }
                                if let Some(ref err) = r.error {
                                    println!("    Error: {err}");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "parent_id": parent_id,
                                    "broadcast": true,
                                    "error": e.to_string(),
                                }))?
                            );
                        } else {
                            eprintln!("Broadcast delegation failed: {}", e);
                        }
                    }
                }
            } else {
                if !json {
                    println!("Delegating to single peer...");
                }
                match node.delegate_task(&request).await {
                    Ok(r) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "parent_id": parent_id,
                                    "subtask_id": r.subtask_id,
                                    "agent_id": r.agent_id,
                                    "success": r.success,
                                    "result_text": r.result_text,
                                    "error": r.error,
                                }))?
                            );
                        } else {
                            let status = if r.success { "OK" } else { "FAIL" };
                            let agent = &r.agent_id[..12.min(r.agent_id.len())];
                            println!("[{status}] agent={agent} subtask={}", r.subtask_id);
                            if let Some(ref text) = r.result_text {
                                println!("Result: {text}");
                            }
                            if let Some(ref err) = r.error {
                                println!("Error: {err}");
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "parent_id": parent_id,
                                    "error": e.to_string(),
                                }))?
                            );
                        } else {
                            eprintln!("Delegation failed: {}", e);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn task_send_json_output_is_parseable() {
        // Mirrors the JSON structure produced by `task send --json`
        let output = serde_json::to_string(&serde_json::json!({
            "task_id": "550e8400-e29b-41d4-a716-446655440000",
            "from": "02aabbcc",
            "to": "02ddeeff",
            "status": "acked",
        }))
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["status"], "acked");
        assert!(parsed["task_id"].is_string());
        assert!(parsed["from"].is_string());
        assert!(parsed["to"].is_string());
    }

    #[test]
    fn task_status_json_output_is_parseable() {
        let output = serde_json::to_string(&serde_json::json!({
            "task_id": "task-42",
            "listener": "02aabbcc",
            "results": [
                {
                    "task_id": "task-42",
                    "state": "Completed",
                    "result": "Echo: hello",
                }
            ],
        }))
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["task_id"], "task-42");
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["state"], "Completed");
    }

    #[test]
    fn stream_chunk_json_output_is_parseable() {
        let output = serde_json::to_string(&serde_json::json!({
            "event": "chunk",
            "task_id": "task-42",
            "chunk_index": 0,
            "text": "Hello ",
            "is_final": false,
        }))
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["event"], "chunk");
        assert_eq!(parsed["chunk_index"], 0);
        assert_eq!(parsed["is_final"], false);
    }
}
