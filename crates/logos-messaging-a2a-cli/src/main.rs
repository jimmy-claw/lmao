use anyhow::Result;
use clap::{Parser, Subcommand};
use logos_messaging_a2a_core::Task;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use std::collections::HashSet;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "logos-messaging-a2a",
    about = "A2A protocol over Waku decentralized transport"
)]
struct Cli {
    /// nwaku REST API URL
    #[arg(long, default_value = "http://localhost:8645", global = true)]
    waku: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
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
    /// Presence management (Waku presence broadcasts)
    Presence {
        #[command(subcommand)]
        action: PresenceAction,
    },
}

#[derive(Debug, Subcommand)]
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
        /// Path to a persistent identity keyfile (hex-encoded 32-byte signing key).
        /// If the file does not exist, a new key is generated and saved.
        #[arg(long)]
        keyfile: Option<PathBuf>,
    },
    /// Discover agents on the network
    Discover,
    /// Print this agent's IntroBundle (for sharing out-of-band)
    Bundle,
}

#[derive(Debug, Subcommand)]
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
    /// Follow a task's streaming output
    Stream {
        /// Task ID (UUID) to follow
        #[arg(long)]
        id: String,
        /// How long to wait for the stream to complete (seconds)
        #[arg(long, default_value = "30")]
        timeout: u64,
    },
}

#[derive(Debug, Subcommand)]
enum PresenceAction {
    /// Announce this agent on the presence topic
    Announce {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Comma-separated capabilities
        #[arg(long, default_value = "text")]
        capabilities: String,
        /// TTL in seconds
        #[arg(long, default_value = "300")]
        ttl: u64,
        /// Keep re-announcing every ttl/2 seconds
        #[arg(long)]
        repeat: bool,
        /// Generate encrypted identity (X25519+ChaCha20-Poly1305)
        #[arg(long)]
        encrypt: bool,
    },
    /// Listen for presence announcements
    Discover {
        /// Filter by capability
        #[arg(long)]
        capability: Option<String>,
        /// Keep listening instead of one-shot
        #[arg(long)]
        watch: bool,
        /// How long to listen in one-shot mode (seconds)
        #[arg(long, default_value = "10")]
        timeout: u64,
    },
    /// Discover and list unique peers (deduplicated)
    Peers {
        /// Filter by capability
        #[arg(long)]
        capability: Option<String>,
        /// Keep listening instead of one-shot
        #[arg(long)]
        watch: bool,
        /// How long to listen in one-shot mode (seconds)
        #[arg(long, default_value = "10")]
        timeout: u64,
    },
}

fn print_peer(agent_id: &str, info: &logos_messaging_a2a_node::presence::PeerInfo) {
    let expired = if info.is_expired() { " [EXPIRED]" } else { "" };
    println!("  Name:         {}", info.name);
    println!("  Capabilities: {}", info.capabilities.join(", "));
    println!("  Pubkey:       {}", agent_id);
    println!("  Waku topic:   {}", info.waku_topic);
    println!("  Last seen:    {}", info.last_seen);
    println!("  Status:       TTL {}s{}", info.ttl_secs, expired);
    println!();
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let transport = LogosMessagingTransport::new(&cli.waku);

    match cli.command {
        Commands::Agent { action } => match action {
            AgentAction::Run {
                name,
                capabilities,
                encrypt,
                keyfile,
            } => {
                let caps: Vec<String> = capabilities
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                let node = if let Some(path) = keyfile {
                    println!("Using keyfile: {}", path.display());
                    WakuA2ANode::from_keyfile(
                        &name,
                        &format!("{} agent", name),
                        caps,
                        transport,
                        &path,
                    )?
                } else if encrypt {
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
            TaskAction::Stream { id, timeout } => {
                let node = WakuA2ANode::new("cli-stream", "CLI client", vec![], transport);
                println!(
                    "Following stream for task {} (timeout {}s)...\n",
                    id, timeout
                );

                let deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
                let mut last_index: Option<u32> = None;

                while tokio::time::Instant::now() < deadline {
                    match node.poll_stream_chunks(&id).await {
                        Ok(chunks) => {
                            for chunk in &chunks {
                                // Only print chunks we haven't printed yet
                                if last_index.is_none() || chunk.chunk_index > last_index.unwrap() {
                                    print!("{}", chunk.text);
                                    last_index = Some(chunk.chunk_index);
                                }
                            }
                            // Check if stream is complete
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
        },
        Commands::Presence { action } => match action {
            PresenceAction::Announce {
                name,
                capabilities,
                ttl,
                repeat,
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

                println!("Announcing presence: {}", node.card.name);
                println!("Pubkey: {}", node.pubkey());
                println!("TTL: {}s", ttl);
                if encrypt {
                    println!("Encryption: ENABLED");
                }

                match node.announce_presence_with_ttl(ttl).await {
                    Ok(()) => println!("Presence announced."),
                    Err(e) => {
                        eprintln!("Announce failed (is nwaku running?): {}", e);
                        return Ok(());
                    }
                }

                if repeat {
                    let interval = std::cmp::max(ttl / 2, 1);
                    println!("Re-announcing every {}s (Ctrl-C to stop)\n", interval);
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                        match node.announce_presence_with_ttl(ttl).await {
                            Ok(()) => println!("Re-announced presence."),
                            Err(e) => eprintln!("Re-announce failed: {}", e),
                        }
                    }
                }
            }
            PresenceAction::Discover {
                capability,
                watch,
                timeout,
            } => {
                let node = WakuA2ANode::new("presence-discover", "temporary", vec![], transport);

                if watch {
                    println!("Watching for presence announcements (Ctrl-C to stop)...\n");
                    let mut seen = HashSet::new();
                    loop {
                        match node.poll_presence().await {
                            Ok(count) => {
                                if count > 0 {
                                    let peers = match &capability {
                                        Some(cap) => node.find_peers_by_capability(cap),
                                        None => node.peers().all_live(),
                                    };
                                    for (id, info) in &peers {
                                        if seen.insert(format!("{}-{}", id, info.last_seen)) {
                                            print_peer(id, info);
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
                } else {
                    println!("Listening for presence announcements ({}s)...\n", timeout);
                    let deadline =
                        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
                    while tokio::time::Instant::now() < deadline {
                        if let Err(e) = node.poll_presence().await {
                            eprintln!("Poll error (is nwaku running?): {}", e);
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }

                    let peers = match &capability {
                        Some(cap) => node.find_peers_by_capability(cap),
                        None => node.peers().all_live(),
                    };

                    if peers.is_empty() {
                        println!("No peers found.");
                    } else {
                        println!("Found {} peer(s):\n", peers.len());
                        for (id, info) in &peers {
                            print_peer(id, info);
                        }
                    }
                }
            }
            PresenceAction::Peers {
                capability,
                watch,
                timeout,
            } => {
                let node = WakuA2ANode::new("presence-peers", "temporary", vec![], transport);

                if watch {
                    println!("Watching for unique peers (Ctrl-C to stop)...\n");
                    let mut known_ids = HashSet::new();
                    loop {
                        match node.poll_presence().await {
                            Ok(count) => {
                                if count > 0 {
                                    let peers = match &capability {
                                        Some(cap) => node.find_peers_by_capability(cap),
                                        None => node.peers().all_live(),
                                    };
                                    for (id, info) in &peers {
                                        if known_ids.insert(id.clone()) {
                                            print_peer(id, info);
                                        }
                                    }
                                    println!("--- {} unique peer(s) ---\n", known_ids.len());
                                }
                            }
                            Err(e) => {
                                eprintln!("Poll error (is nwaku running?): {}", e);
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                } else {
                    println!("Discovering unique peers ({}s)...\n", timeout);
                    let deadline =
                        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
                    while tokio::time::Instant::now() < deadline {
                        if let Err(e) = node.poll_presence().await {
                            eprintln!("Poll error (is nwaku running?): {}", e);
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }

                    let peers = match &capability {
                        Some(cap) => node.find_peers_by_capability(cap),
                        None => node.peers().all_live(),
                    };

                    if peers.is_empty() {
                        println!("No peers found.");
                    } else {
                        println!("Found {} unique peer(s):\n", peers.len());
                        for (id, info) in &peers {
                            print_peer(id, info);
                        }
                    }
                }
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    // ── Presence Announce ──

    #[test]
    fn presence_announce_requires_name() {
        let err = try_parse(&["cli", "presence", "announce"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn presence_announce_with_name() {
        let cli = try_parse(&["cli", "presence", "announce", "--name", "echo"]).unwrap();
        match cli.command {
            Commands::Presence {
                action:
                    PresenceAction::Announce {
                        name,
                        capabilities,
                        ttl,
                        repeat,
                        encrypt,
                    },
            } => {
                assert_eq!(name, "echo");
                assert_eq!(capabilities, "text");
                assert_eq!(ttl, 300);
                assert!(!repeat);
                assert!(!encrypt);
            }
            _ => panic!("expected Presence Announce"),
        }
    }

    #[test]
    fn presence_announce_all_flags() {
        let cli = try_parse(&[
            "cli",
            "presence",
            "announce",
            "--name",
            "bot",
            "--capabilities",
            "text,code",
            "--ttl",
            "600",
            "--repeat",
            "--encrypt",
        ])
        .unwrap();
        match cli.command {
            Commands::Presence {
                action:
                    PresenceAction::Announce {
                        name,
                        capabilities,
                        ttl,
                        repeat,
                        encrypt,
                    },
            } => {
                assert_eq!(name, "bot");
                assert_eq!(capabilities, "text,code");
                assert_eq!(ttl, 600);
                assert!(repeat);
                assert!(encrypt);
            }
            _ => panic!("expected Presence Announce"),
        }
    }

    // ── Presence Discover ──

    #[test]
    fn presence_discover_defaults() {
        let cli = try_parse(&["cli", "presence", "discover"]).unwrap();
        match cli.command {
            Commands::Presence {
                action:
                    PresenceAction::Discover {
                        capability,
                        watch,
                        timeout,
                    },
            } => {
                assert!(capability.is_none());
                assert!(!watch);
                assert_eq!(timeout, 10);
            }
            _ => panic!("expected Presence Discover"),
        }
    }

    #[test]
    fn presence_discover_with_filters() {
        let cli = try_parse(&[
            "cli",
            "presence",
            "discover",
            "--capability",
            "code",
            "--watch",
            "--timeout",
            "30",
        ])
        .unwrap();
        match cli.command {
            Commands::Presence {
                action:
                    PresenceAction::Discover {
                        capability,
                        watch,
                        timeout,
                    },
            } => {
                assert_eq!(capability.as_deref(), Some("code"));
                assert!(watch);
                assert_eq!(timeout, 30);
            }
            _ => panic!("expected Presence Discover"),
        }
    }

    // ── Presence Peers ──

    #[test]
    fn presence_peers_defaults() {
        let cli = try_parse(&["cli", "presence", "peers"]).unwrap();
        match cli.command {
            Commands::Presence {
                action:
                    PresenceAction::Peers {
                        capability,
                        watch,
                        timeout,
                    },
            } => {
                assert!(capability.is_none());
                assert!(!watch);
                assert_eq!(timeout, 10);
            }
            _ => panic!("expected Presence Peers"),
        }
    }

    #[test]
    fn presence_peers_with_filters() {
        let cli = try_parse(&[
            "cli",
            "presence",
            "peers",
            "--capability",
            "summarize",
            "--timeout",
            "20",
        ])
        .unwrap();
        match cli.command {
            Commands::Presence {
                action:
                    PresenceAction::Peers {
                        capability,
                        watch,
                        timeout,
                    },
            } => {
                assert_eq!(capability.as_deref(), Some("summarize"));
                assert!(!watch);
                assert_eq!(timeout, 20);
            }
            _ => panic!("expected Presence Peers"),
        }
    }

    // ── Global --waku flag ──

    #[test]
    fn global_waku_flag_with_presence() {
        let cli = try_parse(&[
            "cli",
            "--waku",
            "http://custom:9090",
            "presence",
            "discover",
        ])
        .unwrap();
        assert_eq!(cli.waku, "http://custom:9090");
    }

    // ── Existing commands still parse ──

    #[test]
    fn agent_run_still_parses() {
        let cli = try_parse(&["cli", "agent", "run", "--name", "test"]).unwrap();
        matches!(cli.command, Commands::Agent { .. });
    }

    #[test]
    fn task_send_still_parses() {
        let cli = try_parse(&["cli", "task", "send", "--to", "abc", "--text", "hello"]).unwrap();
        matches!(cli.command, Commands::Task { .. });
    }

    // ── Default waku URL ──

    #[test]
    fn default_waku_url() {
        let cli = try_parse(&["cli", "agent", "discover"]).unwrap();
        assert_eq!(cli.waku, "http://localhost:8645");
    }

    // ── Agent Discover ──

    #[test]
    fn agent_discover_parses() {
        let cli = try_parse(&["cli", "agent", "discover"]).unwrap();
        match cli.command {
            Commands::Agent {
                action: AgentAction::Discover,
            } => {}
            _ => panic!("expected Agent Discover"),
        }
    }

    // ── Agent Bundle ──

    #[test]
    fn agent_bundle_parses() {
        let cli = try_parse(&["cli", "agent", "bundle"]).unwrap();
        match cli.command {
            Commands::Agent {
                action: AgentAction::Bundle,
            } => {}
            _ => panic!("expected Agent Bundle"),
        }
    }

    // ── Agent Run details ──

    #[test]
    fn agent_run_defaults() {
        let cli = try_parse(&["cli", "agent", "run", "--name", "echo"]).unwrap();
        match cli.command {
            Commands::Agent {
                action:
                    AgentAction::Run {
                        name,
                        capabilities,
                        encrypt,
                        keyfile,
                    },
            } => {
                assert_eq!(name, "echo");
                assert_eq!(capabilities, "text");
                assert!(!encrypt);
                assert!(keyfile.is_none());
            }
            _ => panic!("expected Agent Run"),
        }
    }

    #[test]
    fn agent_run_with_encrypt() {
        let cli = try_parse(&["cli", "agent", "run", "--name", "secure", "--encrypt"]).unwrap();
        match cli.command {
            Commands::Agent {
                action: AgentAction::Run { encrypt, .. },
            } => {
                assert!(encrypt);
            }
            _ => panic!("expected Agent Run"),
        }
    }

    #[test]
    fn agent_run_with_keyfile() {
        let cli = try_parse(&[
            "cli",
            "agent",
            "run",
            "--name",
            "persistent",
            "--keyfile",
            "/tmp/agent.key",
        ])
        .unwrap();
        match cli.command {
            Commands::Agent {
                action:
                    AgentAction::Run {
                        name,
                        keyfile,
                        encrypt,
                        ..
                    },
            } => {
                assert_eq!(name, "persistent");
                assert_eq!(keyfile, Some(PathBuf::from("/tmp/agent.key")));
                assert!(!encrypt);
            }
            _ => panic!("expected Agent Run"),
        }
    }

    // ── Task Send details ──

    #[test]
    fn task_send_fields() {
        let cli = try_parse(&[
            "cli",
            "task",
            "send",
            "--to",
            "abcdef",
            "--text",
            "hello world",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action: TaskAction::Send { to, text },
            } => {
                assert_eq!(to, "abcdef");
                assert_eq!(text, "hello world");
            }
            _ => panic!("expected Task Send"),
        }
    }

    #[test]
    fn task_send_missing_to() {
        let err = try_parse(&["cli", "task", "send", "--text", "hi"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn task_send_missing_text() {
        let err = try_parse(&["cli", "task", "send", "--to", "abc"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    // ── Task Status ──

    #[test]
    fn task_status_parses() {
        let cli = try_parse(&[
            "cli",
            "task",
            "status",
            "--id",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action: TaskAction::Status { id },
            } => {
                assert_eq!(id, "550e8400-e29b-41d4-a716-446655440000");
            }
            _ => panic!("expected Task Status"),
        }
    }

    #[test]
    fn task_status_missing_id() {
        let err = try_parse(&["cli", "task", "status"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    // ── Missing subcommand ──

    #[test]
    fn missing_subcommand() {
        let err = try_parse(&["cli"]).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn unknown_flag_rejected() {
        let err = try_parse(&["cli", "--bogus"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    // ── Task Stream ──

    #[test]
    fn task_stream_with_id() {
        let cli = try_parse(&["cli", "task", "stream", "--id", "task-42"]).unwrap();
        match cli.command {
            Commands::Task {
                action: TaskAction::Stream { id, timeout },
            } => {
                assert_eq!(id, "task-42");
                assert_eq!(timeout, 30); // default
            }
            _ => panic!("expected Task Stream"),
        }
    }

    #[test]
    fn task_stream_with_timeout() {
        let cli = try_parse(&[
            "cli",
            "task",
            "stream",
            "--id",
            "task-42",
            "--timeout",
            "60",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action: TaskAction::Stream { id, timeout },
            } => {
                assert_eq!(id, "task-42");
                assert_eq!(timeout, 60);
            }
            _ => panic!("expected Task Stream"),
        }
    }

    #[test]
    fn task_stream_missing_id() {
        let err = try_parse(&["cli", "task", "stream"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }
}
