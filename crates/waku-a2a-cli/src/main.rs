use anyhow::Result;
use clap::{Parser, Subcommand};
use waku_a2a_core::Task;
use waku_a2a_node::WakuA2ANode;
use waku_a2a_transport::nwaku_rest::NwakuTransport;

#[derive(Parser)]
#[command(
    name = "waku-a2a",
    about = "A2A protocol over Waku decentralized transport"
)]
struct Cli {
    /// nwaku REST API URL
    #[arg(long, default_value = "http://localhost:8645", global = true)]
    waku: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Agent management
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Task management
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
}

#[derive(Subcommand)]
enum AgentAction {
    /// Run an agent that processes incoming tasks
    Run {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Comma-separated capabilities
        #[arg(long, default_value = "text")]
        capabilities: String,
        /// Enable X25519+ChaCha20-Poly1305 encryption
        #[arg(long)]
        encrypt: bool,
    },
    /// Discover agents on the network
    Discover,
    /// Print this agent's IntroBundle (for sharing out-of-band)
    Bundle,
}

#[derive(Subcommand)]
enum TaskAction {
    /// Send a task to an agent
    Send {
        /// Recipient agent public key (hex)
        #[arg(long)]
        to: String,
        /// Text message to send
        #[arg(long)]
        text: String,
    },
    /// Check task status / poll for response
    Status {
        /// Task ID (UUID)
        #[arg(long)]
        id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let transport = NwakuTransport::new(&cli.waku);

    match cli.command {
        Commands::Agent { action } => match action {
            AgentAction::Run {
                name,
                capabilities,
                encrypt,
            } => {
                let caps: Vec<String> = capabilities
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                let node = if encrypt {
                    WakuA2ANode::new_encrypted(&name, &format!("{} agent", name), caps, transport)
                } else {
                    WakuA2ANode::new(&name, &format!("{} agent", name), caps, transport)
                };
                println!("Agent: {}", node.card.name);
                println!("Pubkey: {}", node.pubkey());
                if encrypt {
                    let bundle = node.card.intro_bundle.as_ref().unwrap();
                    println!("Encryption: ENABLED (X25519+ChaCha20-Poly1305)");
                    println!("X25519 pubkey: {}", bundle.agent_pubkey);
                }
                println!("Listening for tasks...\n");

                // Announce on startup
                if let Err(e) = node.announce().await {
                    eprintln!("Warning: announce failed (is nwaku running?): {}", e);
                }

                // Poll loop
                loop {
                    match node.poll_tasks().await {
                        Ok(tasks) => {
                            for task in tasks {
                                println!("Received task {} from {}", task.id, task.from);
                                if let Some(text) = task.text() {
                                    println!("  Message: {}", text);
                                    // Echo behavior by default
                                    let response = format!("Echo: {}", text);
                                    if let Err(e) = node.respond(&task, &response).await {
                                        eprintln!("  Failed to respond: {}", e);
                                    } else {
                                        println!("  Responded: {}", response);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Poll error (is nwaku running?): {}", e);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
            AgentAction::Discover => {
                let node = WakuA2ANode::new("discovery-client", "temporary", vec![], transport);
                match node.discover().await {
                    Ok(cards) => {
                        if cards.is_empty() {
                            println!("No agents found. (Are agents announcing on the network?)");
                        } else {
                            println!("Discovered {} agent(s):\n", cards.len());
                            for card in cards {
                                println!("  Name: {}", card.name);
                                println!("  Description: {}", card.description);
                                println!("  Capabilities: {}", card.capabilities.join(", "));
                                println!("  Pubkey: {}", card.public_key);
                                if let Some(ref bundle) = card.intro_bundle {
                                    println!("  Encryption: YES (X25519: {})", bundle.agent_pubkey);
                                }
                                println!();
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Discovery failed (is nwaku running?): {}", e);
                    }
                }
            }
            AgentAction::Bundle => {
                let node = WakuA2ANode::new_encrypted("bundle-gen", "temporary", vec![], transport);
                let bundle = node.card.intro_bundle.as_ref().unwrap();
                let json = serde_json::to_string_pretty(bundle)?;
                println!("{}", json);
            }
        },
        Commands::Task { action } => match action {
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
                // Poll the sender's task topic for responses
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
        },
    }

    Ok(())
}
