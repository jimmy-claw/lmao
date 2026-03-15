//! Task delegation example: multi-agent subtask forwarding.
//!
//! Demonstrates an orchestrator agent that decomposes a task into subtasks
//! and delegates them to specialist peers discovered via presence — all
//! peer-to-peer over an in-memory transport with no external dependencies.
//!
//! Usage:
//!   cargo run --example task_delegation

use anyhow::Result;
use logos_messaging_a2a::{DelegationRequest, DelegationStrategy, InMemoryTransport, WakuA2ANode};

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== LMAO Task Delegation: Multi-Agent Subtask Forwarding ===\n");

    // ── 1. Create three agents on the same in-memory transport ───────────
    let transport = InMemoryTransport::new();

    let orchestrator = WakuA2ANode::new(
        "orchestrator",
        "Orchestrator: decomposes and delegates tasks",
        vec!["orchestration".to_string()],
        transport.clone(),
    );
    let summarizer = WakuA2ANode::new(
        "summarizer",
        "Summarizer: produces summaries",
        vec!["summarize".to_string()],
        transport.clone(),
    );
    let translator = WakuA2ANode::new(
        "translator",
        "Translator: translates text",
        vec!["translate".to_string()],
        transport.clone(),
    );

    println!("Created orchestrator ({}...)", &orchestrator.pubkey()[..16]);
    println!("Created summarizer   ({}...)", &summarizer.pubkey()[..16]);
    println!("Created translator   ({}...)\n", &translator.pubkey()[..16]);

    // ── 2. All agents broadcast presence ─────────────────────────────────
    orchestrator.announce_presence().await?;
    summarizer.announce_presence().await?;
    translator.announce_presence().await?;
    println!("[all]          Announced presence\n");

    // ── 3. Orchestrator discovers peers ──────────────────────────────────
    orchestrator.poll_presence().await?;

    let all_peers = orchestrator.peers().all_live();
    println!("[orchestrator] Discovered {} peer(s):", all_peers.len());
    for (id, info) in &all_peers {
        println!(
            "               -> {} ({}...) capabilities: {:?}",
            info.name,
            &id[..16],
            info.capabilities
        );
    }
    println!();

    // ── 4. Ensure workers are subscribed before delegation ───────────────
    //
    // In a real network, workers would already be polling. In this
    // synchronous demo we trigger an initial poll so that the in-memory
    // transport creates the subscription channel.
    summarizer.poll_tasks().await?;
    translator.poll_tasks().await?;

    // ── 5. Spawn worker loops that respond to incoming tasks ─────────────
    //
    // Each worker runs in a background task, polling for incoming subtasks
    // and responding with a simulated result.
    let summarizer_handle = {
        let transport = transport.clone();
        let _ = transport; // keep transport alive
        tokio::spawn(async move {
            loop {
                let tasks = summarizer.poll_tasks().await.unwrap();
                for task in &tasks {
                    let text = task.text().unwrap_or("");
                    println!(
                        "[summarizer]   Received subtask {}: \"{}\"",
                        &task.id[..8],
                        text
                    );
                    let summary = format!("Summary of: {text}");
                    summarizer.respond(task, &summary).await.unwrap();
                    println!("[summarizer]   Responded: \"{summary}\"");
                }
                if !tasks.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
    };

    let translator_handle = {
        let transport = transport.clone();
        let _ = transport;
        tokio::spawn(async move {
            loop {
                let tasks = translator.poll_tasks().await.unwrap();
                for task in &tasks {
                    let text = task.text().unwrap_or("");
                    println!(
                        "[translator]   Received subtask {}: \"{}\"",
                        &task.id[..8],
                        text
                    );
                    let translation = format!("Translated: {text}");
                    translator.respond(task, &translation).await.unwrap();
                    println!("[translator]   Responded: \"{translation}\"");
                }
                if !tasks.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
    };

    // ── 6. Orchestrator delegates subtasks ────────────────────────────────

    // 6a. Delegate a summarization subtask (capability-based routing)
    println!("[orchestrator] Delegating summarization subtask...");
    let summarize_request = DelegationRequest {
        parent_task_id: "parent-001".to_string(),
        subtask_text: "The LMAO protocol enables decentralized agent communication.".to_string(),
        strategy: DelegationStrategy::CapabilityMatch {
            capability: "summarize".to_string(),
        },
        timeout_secs: 5,
    };
    let summarize_result = orchestrator.delegate_task(&summarize_request).await?;
    println!(
        "[orchestrator] Summarization result: success={}, text={:?}\n",
        summarize_result.success, summarize_result.result_text
    );

    // 6b. Delegate a translation subtask (capability-based routing)
    println!("[orchestrator] Delegating translation subtask...");
    let translate_request = DelegationRequest {
        parent_task_id: "parent-001".to_string(),
        subtask_text: "Hello, world!".to_string(),
        strategy: DelegationStrategy::CapabilityMatch {
            capability: "translate".to_string(),
        },
        timeout_secs: 5,
    };
    let translate_result = orchestrator.delegate_task(&translate_request).await?;
    println!(
        "[orchestrator] Translation result: success={}, text={:?}\n",
        translate_result.success, translate_result.result_text
    );

    // ── 7. Wait for workers to finish ────────────────────────────────────
    let _ = summarizer_handle.await;
    let _ = translator_handle.await;

    println!("Done! Orchestrator delegated subtasks to specialist agents via presence discovery.");
    Ok(())
}
