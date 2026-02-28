//! Echo agent example.
//!
//! Starts an A2A agent that echoes back any text messages it receives.
//!
//! Usage:
//!   cargo run --example echo_agent
//!   cargo run --example echo_agent -- --waku http://localhost:8645
//!   cargo run --example echo_agent -- --encrypt

use anyhow::Result;
use waku_a2a::{NwakuTransport, WakuA2ANode};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let waku_url = args
        .iter()
        .position(|a| a == "--waku")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "http://localhost:8645".to_string());

    let encrypt = args.iter().any(|a| a == "--encrypt");

    let transport = NwakuTransport::new(&waku_url);
    let node = if encrypt {
        WakuA2ANode::new_encrypted(
            "echo",
            "Echoes back any text message (encrypted)",
            vec!["text".to_string()],
            transport,
        )
    } else {
        WakuA2ANode::new(
            "echo",
            "Echoes back any text message",
            vec!["text".to_string()],
            transport,
        )
    };

    println!("=== Echo Agent ===");
    println!("Name:   {}", node.card.name);
    println!("Pubkey: {}", node.pubkey());
    if let Some(ref bundle) = node.card.intro_bundle {
        println!("Encryption: ENABLED (X25519+ChaCha20-Poly1305)");
        println!("X25519 pubkey: {}", bundle.agent_pubkey);
    }
    println!();

    // Announce presence
    match node.announce().await {
        Ok(()) => println!("Announced on discovery topic."),
        Err(e) => eprintln!("Warning: could not announce (nwaku not running?): {}", e),
    }

    println!("Listening for tasks... (Ctrl+C to stop)\n");

    loop {
        match node.poll_tasks().await {
            Ok(tasks) => {
                for task in &tasks {
                    let text = task.text().unwrap_or("<no text>");
                    println!(
                        "[recv] Task {} from {}",
                        task.id,
                        &task.from[..12.min(task.from.len())]
                    );
                    println!("       Text: {}", text);

                    let response = format!("Echo: {}", text);
                    match node.respond(task, &response).await {
                        Ok(()) => println!("       Replied: {}\n", response),
                        Err(e) => eprintln!("       Reply failed: {}\n", e),
                    }
                }
            }
            Err(e) => {
                // Silently retry — nwaku might not be running
                eprintln!("[poll error] {}", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
