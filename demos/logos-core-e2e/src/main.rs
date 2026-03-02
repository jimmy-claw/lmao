//! Logos Core E2E Demo — two LMAO agents communicating via native Logos Core IPC.
//!
//! This demo exercises `LogosCoreDeliveryTransport` (delivery_module) and
//! `LogosCoreStorageBackend` (storage_module) through the Logos Core C API.
//!
//! When the real Logos Core SDK + plugins are not available, the demo links
//! against a stub `liblogos_core.so` that simulates both modules in-process.
//!
//! Flow:
//!   1. Agent A uploads a large payload → gets CID from storage
//!   2. Agent A sends a task to Agent B referencing the CID
//!   3. Agent B receives the task, downloads payload by CID
//!   4. Agent B processes and responds
//!   5. Agent A receives the response
//!
//! Usage:
//!   cargo run -p logos-core-e2e-demo              # auto-builds stub
//!   LOGOS_CORE_LIB_DIR=/path/to/sdk cargo run -p logos-core-e2e-demo  # real SDK

use std::ffi::CString;
use std::os::raw::c_int;
use std::time::Duration;

use anyhow::{Context, Result};
use logos_messaging_a2a_storage::{LogosCoreStorageBackend, StorageBackend};
use logos_messaging_a2a_transport::LogosCoreDeliveryTransport;
use logos_messaging_a2a_transport::Transport;
use waku_a2a_core::{topics, A2AEnvelope, Task};
use waku_a2a_node::WakuA2ANode;

// ---------------------------------------------------------------------------
// Logos Core lifecycle FFI (init / set_mode / cleanup)
// ---------------------------------------------------------------------------

const LOGOS_MODE_LOCAL: c_int = 0;

extern "C" {
    fn logos_core_init();
    fn logos_core_set_mode(mode: c_int);
    fn logos_core_load_plugin(name: *const i8);
    fn logos_core_cleanup();
}

fn init_logos_core() {
    unsafe {
        logos_core_init();
        logos_core_set_mode(LOGOS_MODE_LOCAL);

        let delivery = CString::new("delivery_module").unwrap();
        let storage = CString::new("storage_module").unwrap();
        logos_core_load_plugin(delivery.as_ptr());
        logos_core_load_plugin(storage.as_ptr());
    }
    println!("[core] Logos Core initialized (headless / local mode)");
    println!("[core] Loaded plugins: delivery_module, storage_module\n");
}

fn cleanup_logos_core() {
    unsafe {
        logos_core_cleanup();
    }
    println!("\n[core] Logos Core cleaned up");
}

/// Yield to the tokio runtime so spawned tasks (e.g. channel bridges) can run.
async fn yield_runtime() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

// ---------------------------------------------------------------------------
// Demo
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  LMAO — Logos Core E2E Demo                                ║");
    println!("║  Transport: LogosCoreDeliveryTransport (delivery_module)    ║");
    println!("║  Storage:   LogosCoreStorageBackend   (storage_module)      ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // 1. Initialize Logos Core headless
    init_logos_core();

    // 2. Create a shared transport for publishing (delivery_module IPC).
    //    Each WakuA2ANode also gets its own transport instance.
    let transport_a = LogosCoreDeliveryTransport::new("{}")
        .await
        .context("failed to create transport A")?;
    println!("[init] Transport A created (delivery_module)");

    let transport_b = LogosCoreDeliveryTransport::new("{}")
        .await
        .context("failed to create transport B")?;
    println!("[init] Transport B created (delivery_module)");

    // 3. Create storage backend (both agents share the storage_module)
    let storage = LogosCoreStorageBackend::new();
    println!("[init] Storage backend created (storage_module)\n");

    // 4. Create LMAO agent nodes
    let agent_a = WakuA2ANode::new(
        "agent-a",
        "Demo agent A — sends tasks with large payloads",
        vec!["text".to_string(), "storage".to_string()],
        transport_a,
    );
    let agent_b = WakuA2ANode::new(
        "agent-b",
        "Demo agent B — receives and processes tasks",
        vec!["text".to_string(), "storage".to_string()],
        transport_b,
    );

    println!("── Agents ──────────────────────────────────────────────────────");
    println!("  Agent A: {} ({}...)", agent_a.card.name, &agent_a.pubkey()[..16]);
    println!("  Agent B: {} ({}...)", agent_b.card.name, &agent_b.pubkey()[..16]);
    println!();

    // 5. Announce both agents
    agent_a.announce().await?;
    agent_b.announce().await?;
    println!();

    // 6. Subscribe both agents to their task topics before any messages are sent.
    //    poll_tasks() lazily subscribes on first call.
    agent_a.poll_tasks().await?;
    agent_b.poll_tasks().await?;

    // 7. Agent A: upload a large payload to storage → get CID
    println!("── Step 1: Agent A uploads payload to Logos Storage ──────────");
    let payload_data = generate_payload();
    println!("  Payload size: {} bytes", payload_data.len());

    let cid = storage
        .upload(payload_data.clone())
        .await
        .map_err(|e| anyhow::anyhow!("storage upload failed: {}", e))?;
    println!("  Uploaded → CID: {}", cid);
    println!();

    // 8. Agent A: send task to Agent B, referencing the CID
    println!("── Step 2: Agent A sends task to Agent B ─────────────────────");
    let task_text = format!(
        "Process payload from storage. CID: {}  (original size: {} bytes)",
        cid,
        payload_data.len()
    );
    let task = Task::new(agent_a.pubkey(), agent_b.pubkey(), &task_text);
    println!("  Task ID: {}", &task.id[..8]);
    println!("  Message: \"{}\"", task_text);

    // Publish directly to B's task topic (bypass SDS ACK wait for demo brevity).
    let envelope = A2AEnvelope::Task(task.clone());
    let envelope_bytes = serde_json::to_vec(&envelope)?;
    let topic_b = topics::task_topic(agent_b.pubkey());

    // Use a fresh transport for sending to avoid borrow issues with agent_a
    let sender = LogosCoreDeliveryTransport::new("{}").await?;
    sender.publish(&topic_b, &envelope_bytes).await?;
    println!("  → Sent via delivery_module IPC");
    println!();

    // Let the tokio bridge tasks forward the events to bounded channels
    yield_runtime().await;

    // 9. Agent B: receive task, download payload from storage
    println!("── Step 3: Agent B receives task + downloads payload ─────────");
    let tasks = agent_b.poll_tasks().await?;
    println!("  Received {} task(s)", tasks.len());

    for t in &tasks {
        let text = t.text().unwrap_or("(no text)");
        println!("  Task {}: \"{}\"", &t.id[..8], text);

        // Extract CID from message and download
        if let Some(cid_start) = text.find("CID: ") {
            let cid_str = &text[cid_start + 5..];
            let cid_end = cid_str.find(' ').unwrap_or(cid_str.len());
            let extracted_cid = &cid_str[..cid_end];
            println!("  Downloading payload CID: {}", extracted_cid);

            let downloaded = storage
                .download(extracted_cid)
                .await
                .map_err(|e| anyhow::anyhow!("storage download failed: {}", e))?;
            println!("  Downloaded {} bytes from storage", downloaded.len());
            assert_eq!(
                downloaded, payload_data,
                "downloaded data should match uploaded data"
            );
            println!("  Payload integrity verified");
        }

        // Respond
        let response = format!("Processed payload ({} bytes). All good!", payload_data.len());
        agent_b.respond(t, &response).await?;
        println!("  → Responded: \"{}\"", response);
    }
    println!();

    // Let the bridge forward B's response to A's channel
    yield_runtime().await;

    // 10. Agent A: receive response
    println!("── Step 4: Agent A receives response ─────────────────────────");
    let responses = agent_a.poll_tasks().await?;
    println!("  Received {} response(s)", responses.len());
    for r in &responses {
        if let Some(text) = r.result_text() {
            println!("  Response: \"{}\"", text);
        }
    }
    println!();

    // 11. Clean up
    println!("══════════════════════════════════════════════════════════════");
    println!("  Demo complete!");
    println!("  Transport: LogosCoreDeliveryTransport (delivery_module IPC)");
    println!("  Storage:   LogosCoreStorageBackend (storage_module IPC)");
    println!("  Payload offloaded to storage via CID, retrieved by recipient");
    println!("══════════════════════════════════════════════════════════════");

    cleanup_logos_core();
    Ok(())
}

/// Generate a synthetic payload (128 KB) to exercise storage offloading.
fn generate_payload() -> Vec<u8> {
    let mut data = Vec::with_capacity(128 * 1024);
    let pattern = b"LMAO-demo-payload-";
    while data.len() < 128 * 1024 {
        data.extend_from_slice(pattern);
    }
    data.truncate(128 * 1024);
    data
}
