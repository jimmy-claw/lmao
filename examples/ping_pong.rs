//! Ping-pong example: two agents exchanging tasks.
//!
//! This example creates two in-process agents using the InMemoryTransport,
//! demonstrating the A2A task flow without requiring a running nwaku node.
//!
//! Usage:
//!   cargo run --example ping_pong
//!   cargo run --example ping_pong -- --encrypt

use anyhow::Result;
use waku_a2a::{A2AEnvelope, AgentIdentity, InMemoryTransport, Task, Transport, WakuA2ANode};

#[tokio::main]
async fn main() -> Result<()> {
    let encrypt = std::env::args().any(|a| a == "--encrypt");

    if encrypt {
        run_encrypted().await
    } else {
        run_plaintext().await
    }
}

async fn run_plaintext() -> Result<()> {
    println!("=== Ping-Pong Demo (plaintext) ===\n");

    let transport = InMemoryTransport::new();

    let ping = WakuA2ANode::new(
        "ping",
        "Sends ping messages",
        vec!["text".to_string()],
        transport.clone(),
    );
    let pong = WakuA2ANode::new(
        "pong",
        "Responds to pings with pongs",
        vec!["text".to_string()],
        transport.clone(),
    );

    println!(
        "Ping agent: {} ({}...)",
        ping.card.name,
        &ping.pubkey()[..16]
    );
    println!(
        "Pong agent: {} ({}...)\n",
        pong.card.name,
        &pong.pubkey()[..16]
    );

    ping.announce().await?;
    pong.announce().await?;

    let discovered = ping.discover().await?;
    println!("Ping discovered {} agent(s)", discovered.len());
    for card in &discovered {
        println!("  -> {} ({}...)", card.name, &card.public_key[..16]);
    }
    println!();

    // Publish task directly (bypasses SDS which would wait for ACK)
    let task = Task::new(ping.pubkey(), pong.pubkey(), "Ping!");
    println!("[ping] Sending: \"Ping!\" (task {})", &task.id[..8]);
    let envelope = A2AEnvelope::Task(task.clone());
    let payload = serde_json::to_vec(&envelope)?;
    let topic = waku_a2a::topics::task_topic(pong.pubkey());
    // Ensure pong is subscribed before we publish
    pong.poll_tasks().await?;
    transport.publish(&topic, &payload).await?;

    let tasks = pong.poll_tasks().await?;
    for t in &tasks {
        let text = t.text().unwrap_or("?");
        println!("[pong] Received: \"{}\" (task {})", text, &t.id[..8]);
        let response = format!("Pong! (reply to: {})", text);
        pong.respond(t, &response).await?;
        println!("[pong] Replied: \"{}\"", response);
    }

    let responses = ping.poll_tasks().await?;
    for r in &responses {
        if let Some(text) = r.result_text() {
            println!("[ping] Got response: \"{}\"", text);
        }
    }

    println!("\nDone! Both agents communicated via in-memory Waku transport.");
    Ok(())
}

async fn run_encrypted() -> Result<()> {
    println!("=== Ping-Pong Demo (encrypted: X25519+ChaCha20-Poly1305) ===\n");

    let transport = InMemoryTransport::new();

    let ping = WakuA2ANode::new_encrypted(
        "ping",
        "Sends encrypted ping messages",
        vec!["text".to_string()],
        transport.clone(),
    );
    let pong = WakuA2ANode::new_encrypted(
        "pong",
        "Responds to encrypted pings",
        vec!["text".to_string()],
        transport.clone(),
    );

    let ping_bundle = ping.card.intro_bundle.as_ref().unwrap();
    let pong_bundle = pong.card.intro_bundle.as_ref().unwrap();
    println!(
        "Ping agent: {} (X25519: {}...)",
        ping.card.name,
        &ping_bundle.agent_pubkey[..16]
    );
    println!(
        "Pong agent: {} (X25519: {}...)\n",
        pong.card.name,
        &pong_bundle.agent_pubkey[..16]
    );

    ping.announce().await?;
    pong.announce().await?;

    let discovered = ping.discover().await?;
    println!("Ping discovered {} agent(s)", discovered.len());
    for card in &discovered {
        let enc = if card.intro_bundle.is_some() {
            "encrypted"
        } else {
            "plaintext"
        };
        println!(
            "  -> {} ({}...) [{}]",
            card.name,
            &card.public_key[..16],
            enc
        );
    }
    println!();

    // Encrypt and publish task directly
    let task = Task::new(ping.pubkey(), pong.pubkey(), "Ping! (encrypted)");
    println!(
        "[ping] Sending encrypted: \"Ping! (encrypted)\" (task {})",
        &task.id[..8]
    );
    let envelope = {
        let pong_card = &pong.card;
        let identity = ping.identity().unwrap();
        let their_pubkey = AgentIdentity::parse_public_key(
            &pong_card.intro_bundle.as_ref().unwrap().agent_pubkey,
        )?;
        let session_key = identity.shared_key(&their_pubkey);
        let task_json = serde_json::to_vec(&task)?;
        let encrypted = session_key.encrypt(&task_json)?;
        A2AEnvelope::EncryptedTask {
            encrypted,
            sender_pubkey: identity.public_key_hex(),
        }
    };
    let payload = serde_json::to_vec(&envelope)?;
    let topic = waku_a2a::topics::task_topic(pong.pubkey());
    pong.poll_tasks().await?;
    transport.publish(&topic, &payload).await?;

    // Pong polls and decrypts
    let tasks = pong.poll_tasks().await?;
    for t in &tasks {
        let text = t.text().unwrap_or("?");
        println!("[pong] Decrypted: \"{}\" (task {})", text, &t.id[..8]);
        let response = format!("Pong! (reply to: {})", text);
        pong.respond_to(t, &response, Some(&ping.card)).await?;
        println!("[pong] Replied (encrypted): \"{}\"", response);
    }

    // Ping polls for encrypted response
    let responses = ping.poll_tasks().await?;
    for r in &responses {
        if let Some(text) = r.result_text() {
            println!("[ping] Decrypted response: \"{}\"", text);
        }
    }

    println!("\nDone! Both agents communicated with end-to-end encryption.");
    Ok(())
}
