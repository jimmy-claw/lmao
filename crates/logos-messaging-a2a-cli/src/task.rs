use anyhow::Result;
use logos_messaging_a2a_core::Task;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::cli::TaskAction;

pub async fn handle(action: TaskAction, transport: LogosMessagingTransport) -> Result<()> {
    match action {
        TaskAction::Send { to, text } => {
            let node = WakuA2ANode::new("cli-sender", "CLI client", vec![], transport);
            println!("Sending task to {}...", &to[..12.min(to.len())]);
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
            let node = WakuA2ANode::new("cli-poller", "CLI client", vec![], transport);
            println!("Polling for task {} responses...", id);
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
            let node = WakuA2ANode::new("cli-stream", "CLI client", vec![], transport);
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
