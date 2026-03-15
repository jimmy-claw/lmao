use anyhow::Result;
use logos_messaging_a2a_core::Task;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::cli::TaskAction;
use crate::common::{build_node, IdentityConfig};

pub async fn handle(
    action: TaskAction,
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
) -> Result<()> {
    match action {
        TaskAction::Send { to, text } => {
            let node = build_node("cli-sender", "CLI client", vec![], transport, identity)?;
            println!("Sending task to {}...", &to[..12.min(to.len())]);
            println!("From pubkey: {}", node.pubkey());
            let task = Task::new(node.pubkey(), &to, &text);
            match node.send_task(&task).await {
                Ok(acked) => {
                    println!("Task ID: {}", task.id);
                    if acked {
                        println!("Status: ACKed by recipient");
                    } else {
                        println!("Status: Sent (no ACK — recipient may be offline)");
                    }
                }
                Err(e) => {
                    eprintln!("Failed to send task: {}", e);
                    println!("Task ID: {} (failed)", task.id);
                }
            }
        }
        TaskAction::Status { id } => {
            let node = build_node("cli-poller", "CLI client", vec![], transport, identity)?;
            println!("Polling for task {} responses...", id);
            println!("Listening as: {}", node.pubkey());
            match node.poll_tasks().await {
                Ok(tasks) => {
                    let found: Vec<_> = tasks.iter().filter(|t| t.id == id).collect();
                    if found.is_empty() {
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
                    eprintln!("Failed to poll: {}", e);
                }
            }
        }
        TaskAction::Stream { id, timeout } => {
            let node = build_node("cli-stream", "CLI client", vec![], transport, identity)?;
            println!(
                "Following stream for task {} (timeout {}s)...\n",
                id, timeout
            );

            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
            let mut last_index: Option<u32> = None;

            while tokio::time::Instant::now() < deadline {
                match node.poll_stream_chunks(&id).await {
                    Ok(chunks) => {
                        for chunk in &chunks {
                            if last_index.is_none() || chunk.chunk_index > last_index.unwrap() {
                                print!("{}", chunk.text);
                                last_index = Some(chunk.chunk_index);
                            }
                        }
                        if chunks.iter().any(|c| c.is_final) {
                            println!();
                            println!("\n--- Stream complete ({} chunks) ---", chunks.len());
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
                println!("No stream chunks received for task {}", id);
            }
        }
    }
    Ok(())
}
