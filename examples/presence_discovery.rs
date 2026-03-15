//! Presence discovery example: full LMAO agent lifecycle.
//!
//! Demonstrates presence broadcast, peer discovery via the peer map,
//! and a complete task round-trip between two agents — all peer-to-peer
//! over an in-memory Waku transport with no external dependencies.
//!
//! Usage:
//!   cargo run --example presence_discovery

use anyhow::Result;
use logos_messaging_a2a::{A2AEnvelope, InMemoryTransport, Task, Transport, WakuA2ANode};

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== LMAO Agent Lifecycle: Presence Discovery ===\n");

    // ── 1. Create two agents sharing the same in-memory transport ───────
    let transport = InMemoryTransport::new();

    let alice = WakuA2ANode::new(
        "alice",
        "Alice: asks questions",
        vec!["question-answering".to_string()],
        transport.clone(),
    );
    let bob = WakuA2ANode::new(
        "bob",
        "Bob: answers questions",
        vec![
            "question-answering".to_string(),
            "summarization".to_string(),
        ],
        transport.clone(),
    );

    println!("Created alice ({}...)", &alice.pubkey()[..16]);
    println!("Created bob   ({}...)\n", &bob.pubkey()[..16]);

    // ── 2. Both agents broadcast signed presence announcements ─────────
    alice.announce_presence().await?;
    println!("[alice] Announced presence");

    bob.announce_presence().await?;
    println!("[bob]   Announced presence\n");

    // ── 3. Both agents poll presence and discover each other ────────────
    let alice_count = alice.poll_presence().await?;
    let bob_count = bob.poll_presence().await?;

    println!(
        "[alice] Polled presence: discovered {} peer(s)",
        alice_count
    );
    for (id, info) in alice.peers().all_live() {
        println!(
            "        -> {} ({}...) capabilities: {:?}",
            info.name,
            &id[..16],
            info.capabilities
        );
    }

    println!("[bob]   Polled presence: discovered {} peer(s)", bob_count);
    for (id, info) in bob.peers().all_live() {
        println!(
            "        -> {} ({}...) capabilities: {:?}",
            info.name,
            &id[..16],
            info.capabilities
        );
    }
    println!();

    // ── 3b. Find peers by capability ────────────────────────────────────
    let summarizers = alice.find_peers_by_capability("summarization");
    println!(
        "[alice] Found {} peer(s) with 'summarization' capability:",
        summarizers.len()
    );
    for (id, info) in &summarizers {
        println!("        -> {} ({}...)", info.name, &id[..16]);
    }
    println!();

    // ── 4. Alice sends a task to Bob ────────────────────────────────────
    //
    // We publish directly to the transport (like the ping_pong example)
    // to avoid the SDS ACK timeout in a synchronous demo. In production
    // code you would use `alice.send_task(&task).await?` for reliable
    // delivery with retransmission.
    let task = Task::new(alice.pubkey(), bob.pubkey(), "What is the LMAO protocol?");
    println!(
        "[alice] Sending task {}: \"{}\"",
        &task.id[..8],
        task.text().unwrap()
    );

    let envelope = A2AEnvelope::Task(task.clone());
    let payload = serde_json::to_vec(&envelope)?;
    let topic = logos_messaging_a2a::topics::task_topic(bob.pubkey());

    // Ensure bob is subscribed before we publish
    bob.poll_tasks().await?;
    transport.publish(&topic, &payload).await?;

    // ── 5. Bob receives the task and responds ───────────────────────────
    let incoming = bob.poll_tasks().await?;
    for t in &incoming {
        let text = t.text().unwrap_or("?");
        println!("[bob]   Received task {}: \"{}\"", &t.id[..8], text);

        let answer = "LMAO (Logos Messaging A2A Orchestration) is a peer-to-peer \
                      agent communication protocol built on Waku.";
        bob.respond(t, answer).await?;
        println!("[bob]   Responded: \"{}\"", answer);
    }

    // ── 6. Alice receives Bob's response — full round-trip ──────────────
    let responses = alice.poll_tasks().await?;
    for r in &responses {
        if let Some(text) = r.result_text() {
            println!("[alice] Got response for task {}: \"{}\"", &r.id[..8], text);
        }
    }

    println!("\nDone! Full lifecycle: presence -> discovery -> task exchange.");
    Ok(())
}
