use clap::{Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "logos-messaging-a2a",
    about = "A2A protocol over Waku decentralized transport"
)]
pub struct Cli {
    /// nwaku REST API URL
    #[arg(long, default_value = "http://localhost:8645", global = true)]
    pub waku: String,

    /// Path to a persistent identity keyfile (hex-encoded 32-byte signing key).
    /// If the file does not exist, a new key is generated and saved.
    /// When provided, all commands share the same identity.
    #[arg(long, global = true)]
    pub keyfile: Option<PathBuf>,

    /// Enable X25519+ChaCha20-Poly1305 encryption for this identity.
    #[arg(long, global = true)]
    pub encrypt: bool,

    /// Output structured JSON instead of human-readable text.
    /// When set, JSON goes to stdout and informational messages go to stderr.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
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
    /// Session management
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Check Waku node connectivity and health
    Health,
    /// Display operational metrics counters
    Metrics,
    /// Generate shell completions
    Completion {
        /// Shell to generate completions for
        shell: Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    /// Run an agent that processes incoming tasks
    Run {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Comma-separated capabilities
        #[arg(long, default_value = "text")]
        capabilities: String,
    },
    /// Discover agents on the network
    Discover,
    /// Print this agent's IntroBundle (for sharing out-of-band)
    Bundle,
}

#[derive(Debug, Subcommand)]
pub enum TaskAction {
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
    /// Delegate a subtask to a peer agent
    Delegate {
        /// Recipient agent public key (hex) — direct delegation
        #[arg(long)]
        to: Option<String>,
        /// Required capability — discovery-based delegation
        #[arg(long)]
        capability: Option<String>,
        /// Text of the subtask to delegate
        #[arg(long)]
        text: String,
        /// Parent task ID this subtask belongs to
        #[arg(long, default_value = "cli")]
        parent_id: String,
        /// Timeout in seconds for the delegation
        #[arg(long, default_value = "30")]
        timeout: u64,
        /// Broadcast to all matching peers instead of just one
        #[arg(long)]
        broadcast: bool,
        /// Delegation strategy: first-available, broadcast, round-robin
        #[arg(long)]
        strategy: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PresenceAction {
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

#[derive(Debug, Subcommand)]
pub enum SessionAction {
    /// List all active sessions
    List,
    /// Show details of a specific session
    Show {
        /// Session ID (UUID)
        #[arg(long)]
        id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    // ── Global identity flags ──

    #[test]
    fn global_keyfile_flag() {
        let cli = try_parse(&["cli", "--keyfile", "/tmp/my.key", "agent", "discover"]).unwrap();
        assert_eq!(cli.keyfile, Some(PathBuf::from("/tmp/my.key")));
        assert!(!cli.encrypt);
    }

    #[test]
    fn global_encrypt_flag() {
        let cli = try_parse(&["cli", "--encrypt", "agent", "discover"]).unwrap();
        assert!(cli.encrypt);
        assert!(cli.keyfile.is_none());
    }

    #[test]
    fn global_keyfile_and_encrypt() {
        let cli = try_parse(&[
            "cli",
            "--keyfile",
            "/tmp/id.key",
            "--encrypt",
            "task",
            "send",
            "--to",
            "abc",
            "--text",
            "hi",
        ])
        .unwrap();
        assert_eq!(cli.keyfile, Some(PathBuf::from("/tmp/id.key")));
        assert!(cli.encrypt);
    }

    #[test]
    fn global_flags_work_after_subcommand() {
        // clap global flags can appear after the subcommand too
        let cli = try_parse(&["cli", "agent", "discover", "--keyfile", "/tmp/late.key"]).unwrap();
        assert_eq!(cli.keyfile, Some(PathBuf::from("/tmp/late.key")));
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
                    },
            } => {
                assert_eq!(name, "echo");
                assert_eq!(capabilities, "text");
                assert_eq!(ttl, 300);
                assert!(!repeat);
            }
            _ => panic!("expected Presence Announce"),
        }
    }

    #[test]
    fn presence_announce_all_flags() {
        let cli = try_parse(&[
            "cli",
            "--encrypt",
            "presence",
            "announce",
            "--name",
            "bot",
            "--capabilities",
            "text,code",
            "--ttl",
            "600",
            "--repeat",
        ])
        .unwrap();
        assert!(cli.encrypt);
        match cli.command {
            Commands::Presence {
                action:
                    PresenceAction::Announce {
                        name,
                        capabilities,
                        ttl,
                        repeat,
                    },
            } => {
                assert_eq!(name, "bot");
                assert_eq!(capabilities, "text,code");
                assert_eq!(ttl, 600);
                assert!(repeat);
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
                action: AgentAction::Run { name, capabilities },
            } => {
                assert_eq!(name, "echo");
                assert_eq!(capabilities, "text");
            }
            _ => panic!("expected Agent Run"),
        }
        assert!(!cli.encrypt);
        assert!(cli.keyfile.is_none());
    }

    #[test]
    fn agent_run_with_global_encrypt() {
        let cli = try_parse(&["cli", "--encrypt", "agent", "run", "--name", "secure"]).unwrap();
        assert!(cli.encrypt);
    }

    #[test]
    fn agent_run_with_global_keyfile() {
        let cli = try_parse(&[
            "cli",
            "--keyfile",
            "/tmp/agent.key",
            "agent",
            "run",
            "--name",
            "persistent",
        ])
        .unwrap();
        assert_eq!(cli.keyfile, Some(PathBuf::from("/tmp/agent.key")));
        assert!(!cli.encrypt);
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

    // ── Task Delegate ──

    #[test]
    fn task_delegate_with_to() {
        let cli = try_parse(&[
            "cli",
            "task",
            "delegate",
            "--to",
            "02abcdef",
            "--text",
            "do something",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action:
                    TaskAction::Delegate {
                        to,
                        capability,
                        text,
                        parent_id,
                        timeout,
                        broadcast,
                        strategy,
                    },
            } => {
                assert_eq!(to, Some("02abcdef".to_string()));
                assert!(capability.is_none());
                assert_eq!(text, "do something");
                assert_eq!(parent_id, "cli");
                assert_eq!(timeout, 30);
                assert!(!broadcast);
                assert!(strategy.is_none());
            }
            _ => panic!("expected Task Delegate"),
        }
    }

    #[test]
    fn task_delegate_with_capability() {
        let cli = try_parse(&[
            "cli",
            "task",
            "delegate",
            "--capability",
            "summarize",
            "--text",
            "summarize this",
            "--parent-id",
            "task-42",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action:
                    TaskAction::Delegate {
                        to,
                        capability,
                        text,
                        parent_id,
                        ..
                    },
            } => {
                assert!(to.is_none());
                assert_eq!(capability, Some("summarize".to_string()));
                assert_eq!(text, "summarize this");
                assert_eq!(parent_id, "task-42");
            }
            _ => panic!("expected Task Delegate"),
        }
    }

    #[test]
    fn task_delegate_broadcast_flag() {
        let cli = try_parse(&[
            "cli",
            "task",
            "delegate",
            "--capability",
            "text",
            "--text",
            "broadcast",
            "--broadcast",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action: TaskAction::Delegate { broadcast, .. },
            } => {
                assert!(broadcast);
            }
            _ => panic!("expected Task Delegate"),
        }
    }

    #[test]
    fn task_delegate_custom_timeout() {
        let cli = try_parse(&[
            "cli",
            "task",
            "delegate",
            "--text",
            "quick",
            "--timeout",
            "5",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action: TaskAction::Delegate { timeout, .. },
            } => {
                assert_eq!(timeout, 5);
            }
            _ => panic!("expected Task Delegate"),
        }
    }

    #[test]
    fn task_delegate_round_robin_strategy() {
        let cli = try_parse(&[
            "cli",
            "task",
            "delegate",
            "--text",
            "round robin task",
            "--strategy",
            "round-robin",
        ])
        .unwrap();
        match cli.command {
            Commands::Task {
                action: TaskAction::Delegate { strategy, text, .. },
            } => {
                assert_eq!(strategy, Some("round-robin".to_string()));
                assert_eq!(text, "round robin task");
            }
            _ => panic!("expected Task Delegate"),
        }
    }

    #[test]
    fn task_delegate_missing_text() {
        let err = try_parse(&["cli", "task", "delegate", "--to", "02ab"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    // ── Session List ──

    #[test]
    fn session_list_parses() {
        let cli = try_parse(&["cli", "session", "list"]).unwrap();
        match cli.command {
            Commands::Session {
                action: SessionAction::List,
            } => {}
            _ => panic!("expected Session List"),
        }
    }

    // ── Session Show ──

    #[test]
    fn session_show_parses() {
        let cli = try_parse(&[
            "cli",
            "session",
            "show",
            "--id",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .unwrap();
        match cli.command {
            Commands::Session {
                action: SessionAction::Show { id },
            } => {
                assert_eq!(id, "550e8400-e29b-41d4-a716-446655440000");
            }
            _ => panic!("expected Session Show"),
        }
    }

    #[test]
    fn session_show_missing_id() {
        let err = try_parse(&["cli", "session", "show"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn session_list_with_global_flags() {
        let cli = try_parse(&[
            "cli",
            "--keyfile",
            "/tmp/s.key",
            "--encrypt",
            "session",
            "list",
        ])
        .unwrap();
        assert_eq!(cli.keyfile, Some(PathBuf::from("/tmp/s.key")));
        assert!(cli.encrypt);
        match cli.command {
            Commands::Session {
                action: SessionAction::List,
            } => {}
            _ => panic!("expected Session List"),
        }
    }

    // ── Global --json flag ──

    #[test]
    fn json_flag_defaults_to_false() {
        let cli = try_parse(&["cli", "agent", "discover"]).unwrap();
        assert!(!cli.json);
    }

    #[test]
    fn json_flag_before_subcommand() {
        let cli = try_parse(&["cli", "--json", "agent", "discover"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn json_flag_after_subcommand() {
        let cli = try_parse(&["cli", "agent", "discover", "--json"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn json_flag_with_task_send() {
        let cli = try_parse(&[
            "cli", "--json", "task", "send", "--to", "abc", "--text", "hi",
        ])
        .unwrap();
        assert!(cli.json);
        match cli.command {
            Commands::Task {
                action: TaskAction::Send { to, text },
            } => {
                assert_eq!(to, "abc");
                assert_eq!(text, "hi");
            }
            _ => panic!("expected Task Send"),
        }
    }

    #[test]
    fn json_flag_with_presence_discover() {
        let cli = try_parse(&["cli", "--json", "presence", "discover"]).unwrap();
        assert!(cli.json);
        match cli.command {
            Commands::Presence {
                action: PresenceAction::Discover { .. },
            } => {}
            _ => panic!("expected Presence Discover"),
        }
    }

    #[test]
    fn json_flag_with_session_list() {
        let cli = try_parse(&["cli", "--json", "session", "list"]).unwrap();
        assert!(cli.json);
        match cli.command {
            Commands::Session {
                action: SessionAction::List,
            } => {}
            _ => panic!("expected Session List"),
        }
    }

    // ── Health ──

    #[test]
    fn health_parses() {
        let cli = try_parse(&["cli", "health"]).unwrap();
        match cli.command {
            Commands::Health => {}
            _ => panic!("expected Health"),
        }
    }

    #[test]
    fn health_with_custom_waku_url() {
        let cli = try_parse(&["cli", "--waku", "http://node:9090", "health"]).unwrap();
        assert_eq!(cli.waku, "http://node:9090");
        match cli.command {
            Commands::Health => {}
            _ => panic!("expected Health"),
        }
    }

    #[test]
    fn health_with_json_flag() {
        let cli = try_parse(&["cli", "--json", "health"]).unwrap();
        assert!(cli.json);
        match cli.command {
            Commands::Health => {}
            _ => panic!("expected Health"),
        }
    }

    // ── Completion ──

    #[test]
    fn completion_bash_parses() {
        let cli = try_parse(&["cli", "completion", "bash"]).unwrap();
        match cli.command {
            Commands::Completion { shell } => {
                assert_eq!(shell, clap_complete::Shell::Bash);
            }
            _ => panic!("expected Completion"),
        }
    }

    #[test]
    fn completion_zsh_parses() {
        let cli = try_parse(&["cli", "completion", "zsh"]).unwrap();
        match cli.command {
            Commands::Completion { shell } => {
                assert_eq!(shell, clap_complete::Shell::Zsh);
            }
            _ => panic!("expected Completion"),
        }
    }

    #[test]
    fn completion_missing_shell_errors() {
        let err = try_parse(&["cli", "completion"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn completion_invalid_shell_errors() {
        let err = try_parse(&["cli", "completion", "nushell"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn json_flag_combined_with_other_globals() {
        let cli = try_parse(&[
            "cli",
            "--json",
            "--encrypt",
            "--keyfile",
            "/tmp/k.key",
            "agent",
            "run",
            "--name",
            "test",
        ])
        .unwrap();
        assert!(cli.json);
        assert!(cli.encrypt);
        assert_eq!(cli.keyfile, Some(PathBuf::from("/tmp/k.key")));
    }
}
