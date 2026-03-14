//! MCP Bridge Server for Logos Messaging A2A
//!
//! Exposes discovered Waku A2A agents as MCP tools.
//! Each agent on the network becomes a callable tool in Claude Desktop, Cursor, etc.
//!
//! Architecture:
//!   MCP Host (Claude) → stdio → logos-messaging-a2a-mcp → Waku → Agent Fleet
//!
//! Usage:
//!   logos-messaging-a2a-mcp --waku-url http://localhost:8645
//!
//! In Claude Desktop's config:
//!   { "mcpServers": { "logos-agents": { "command": "logos-messaging-a2a-mcp", "args": ["--waku-url", "http://..."] } } }

use anyhow::Result;
use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use logos_messaging_a2a_mcp::LogosA2ABridge;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

#[derive(Debug, Parser)]
#[command(
    name = "logos-messaging-a2a-mcp",
    about = "MCP bridge for Logos A2A agents"
)]
struct Cli {
    /// nwaku REST API URL
    #[arg(long, default_value = "http://localhost:8645")]
    waku_url: String,

    /// How long (seconds) to wait for agent responses
    #[arg(long, default_value_t = 30)]
    timeout: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    tracing::info!(
        "Starting Logos A2A MCP bridge (waku: {}, timeout: {}s)",
        cli.waku_url,
        cli.timeout
    );

    let transport = LogosMessagingTransport::new(&cli.waku_url);
    let node = WakuA2ANode::new(
        "mcp-bridge",
        "MCP bridge — proxies tool calls to Logos A2A agents",
        vec!["mcp-bridge".into()],
        transport,
    );
    let bridge = LogosA2ABridge::from_node(node, cli.timeout);

    {
        let node: tokio::sync::RwLockReadGuard<WakuA2ANode<LogosMessagingTransport>> =
            bridge.node.read().await;
        if let Err(e) = node.announce().await {
            tracing::warn!("Failed to announce bridge on network: {e}");
        }
    }

    let service = bridge.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use logos_messaging_a2a_mcp::{
        format_peer_entry, result_text, AgentCard, GetAgentStatusInput, SendToAgentInput,
    };
    use logos_messaging_a2a_transport::memory::InMemoryTransport;
    use logos_messaging_a2a_transport::Transport;
    use rmcp::{handler::server::wrapper::Parameters, model::*, ServerHandler};

    /// Create a bridge backed by InMemoryTransport (no nwaku required).
    fn make_test_bridge(transport: InMemoryTransport) -> LogosA2ABridge<InMemoryTransport> {
        let node = WakuA2ANode::new(
            "mcp-bridge",
            "MCP bridge for tests",
            vec!["mcp-bridge".into()],
            transport,
        );
        LogosA2ABridge::from_node(node, 30)
    }

    /// Create an AgentCard fixture.
    fn make_card(name: &str, desc: &str, caps: &[&str], pubkey: &str) -> AgentCard {
        AgentCard {
            name: name.to_string(),
            version: "1.0".to_string(),
            description: desc.to_string(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            public_key: pubkey.to_string(),
            intro_bundle: None,
        }
    }

    // ── CLI arg parsing ──

    #[test]
    fn cli_defaults() {
        let cli = Cli::try_parse_from(["mcp"]).unwrap();
        assert_eq!(cli.waku_url, "http://localhost:8645");
        assert_eq!(cli.timeout, 30);
    }

    #[test]
    fn cli_custom_waku_url() {
        let cli = Cli::try_parse_from(["mcp", "--waku-url", "http://node:9090"]).unwrap();
        assert_eq!(cli.waku_url, "http://node:9090");
    }

    #[test]
    fn cli_custom_timeout() {
        let cli = Cli::try_parse_from(["mcp", "--timeout", "60"]).unwrap();
        assert_eq!(cli.timeout, 60);
    }

    #[test]
    fn cli_all_flags() {
        let cli =
            Cli::try_parse_from(["mcp", "--waku-url", "http://x:1234", "--timeout", "5"]).unwrap();
        assert_eq!(cli.waku_url, "http://x:1234");
        assert_eq!(cli.timeout, 5);
    }

    #[test]
    fn cli_rejects_unknown_flag() {
        let err = Cli::try_parse_from(["mcp", "--bogus"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn cli_rejects_invalid_timeout() {
        let err = Cli::try_parse_from(["mcp", "--timeout", "not-a-number"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    // ── list_cached_agents ──

    #[tokio::test]
    async fn list_cached_agents_empty_cache() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let result = bridge.list_cached_agents().await.unwrap();
        assert!(result_text(&result).contains("No cached agents"));
    }

    #[tokio::test]
    async fn list_cached_agents_with_agents() {
        let bridge = make_test_bridge(InMemoryTransport::new());

        {
            let mut agents = bridge.agents.write().await;
            agents.push(make_card(
                "test-agent",
                "A test agent",
                &["text"],
                "deadbeef",
            ));
        }

        let result = bridge.list_cached_agents().await.unwrap();
        let text = result_text(&result);
        assert!(text.contains("test-agent"));
        assert!(text.contains("A test agent"));
    }

    // ── discover_agents with mocked transport ──

    #[tokio::test]
    async fn discover_agents_finds_announced_agents() {
        let transport = InMemoryTransport::new();

        // Create an agent that announces on the same transport.
        let echo = WakuA2ANode::new(
            "echo-agent",
            "Echoes messages back",
            vec!["echo".into(), "text".into()],
            transport.clone(),
        );
        echo.announce().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents().await.unwrap();
        let text = result_text(&result);

        // Response should list the discovered agent.
        assert!(text.contains("Found 1 agent(s)"));
        assert!(text.contains("echo-agent"));
        assert!(text.contains("Echoes messages back"));
        assert!(text.contains("echo, text"));

        // Cache should be populated.
        let cached = bridge.agents.read().await;
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].name, "echo-agent");
    }

    #[tokio::test]
    async fn discover_agents_no_agents_found() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let result = bridge.discover_agents().await.unwrap();
        let text = result_text(&result);
        assert!(text.contains("No agents found"));
    }

    #[tokio::test]
    async fn discover_agents_multiple_agents() {
        let transport = InMemoryTransport::new();

        // Announce three agents on the shared transport.
        for (name, desc, caps) in [
            ("summarizer", "Summarizes text", vec!["summarize"]),
            ("translator", "Translates text", vec!["translate"]),
            ("coder", "Writes code", vec!["code", "text"]),
        ] {
            let node = WakuA2ANode::new(
                name,
                desc,
                caps.into_iter().map(String::from).collect(),
                transport.clone(),
            );
            node.announce().await.unwrap();
        }

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents().await.unwrap();
        let text = result_text(&result);

        assert!(text.contains("Found 3 agent(s)"));
        assert!(text.contains("summarizer"));
        assert!(text.contains("translator"));
        assert!(text.contains("coder"));

        // All three should be cached.
        let cached = bridge.agents.read().await;
        assert_eq!(cached.len(), 3);
    }

    #[tokio::test]
    async fn discover_agents_cache_is_replaced_on_rediscovery() {
        let transport = InMemoryTransport::new();

        // First discovery: one agent.
        let agent1 = WakuA2ANode::new("agent-1", "First", vec!["a".into()], transport.clone());
        agent1.announce().await.unwrap();

        let bridge = make_test_bridge(transport.clone());
        bridge.discover_agents().await.unwrap();
        assert_eq!(bridge.agents.read().await.len(), 1);

        // Second discovery: the bridge re-discovers and gets agent-1 again
        // (InMemoryTransport replays history). Cache should reflect the new snapshot.
        let result = bridge.discover_agents().await.unwrap();
        let text = result_text(&result);
        assert!(text.contains("agent-1"));
    }

    // ── discover_agents response formatting ──

    #[tokio::test]
    async fn discover_agents_format_includes_version_and_pubkey() {
        let transport = InMemoryTransport::new();

        let agent = WakuA2ANode::new(
            "format-check",
            "Checks formatting",
            vec!["test".into()],
            transport.clone(),
        );
        agent.announce().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents().await.unwrap();
        let text = result_text(&result);

        // Should contain version and truncated public key.
        assert!(text.contains("(v0.1.0)"));
        assert!(text.contains("Public key:"));
        assert!(text.contains("..."));
    }

    // ── send_to_agent ──

    #[tokio::test]
    async fn send_to_agent_unknown_agent_returns_error() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let err = bridge
            .send_to_agent(Parameters(SendToAgentInput {
                agent_name: "nonexistent".to_string(),
                message: "hello".to_string(),
            }))
            .await
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("nonexistent"));
        assert!(err.message.contains("not found"));
    }

    #[tokio::test]
    async fn send_to_agent_suggests_discover_on_miss() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let err = bridge
            .send_to_agent(Parameters(SendToAgentInput {
                agent_name: "missing".to_string(),
                message: "hi".to_string(),
            }))
            .await
            .unwrap_err();

        assert!(err.message.contains("discover_agents"));
    }

    // ── SendToAgentInput deserialization ──

    #[test]
    fn send_to_agent_input_deserializes() {
        let json = r#"{"agent_name": "echo", "message": "hello world"}"#;
        let input: SendToAgentInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.agent_name, "echo");
        assert_eq!(input.message, "hello world");
    }

    #[test]
    fn send_to_agent_input_rejects_missing_fields() {
        let json = r#"{"agent_name": "echo"}"#;
        assert!(serde_json::from_str::<SendToAgentInput>(json).is_err());

        let json = r#"{"message": "hello"}"#;
        assert!(serde_json::from_str::<SendToAgentInput>(json).is_err());
    }

    #[test]
    fn send_to_agent_input_accepts_extra_fields() {
        let json = r#"{"agent_name": "echo", "message": "hi", "extra": "ignored"}"#;
        let input: SendToAgentInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.agent_name, "echo");
    }

    // ── ServerInfo / capabilities ──

    #[test]
    fn server_info_has_instructions() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let info = bridge.get_info();

        let instructions = info.instructions.expect("should have instructions");
        assert!(instructions.contains("Logos"));
        assert!(instructions.contains("discover_agents"));
    }

    #[test]
    fn server_info_enables_tools() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let info = bridge.get_info();

        assert!(
            info.capabilities.tools.is_some(),
            "tools capability should be enabled"
        );
    }

    // ── Multiple agents in cache formatting ──

    #[tokio::test]
    async fn list_cached_agents_multiple_formatting() {
        let bridge = make_test_bridge(InMemoryTransport::new());

        {
            let mut agents = bridge.agents.write().await;
            agents.push(make_card(
                "summarizer",
                "Summarizes documents",
                &["summarize"],
                "aabbccdd",
            ));
            agents.push(make_card(
                "translator",
                "Translates text between languages",
                &["translate"],
                "11223344",
            ));
            agents.push(make_card(
                "coder",
                "Writes and reviews code",
                &["code", "review"],
                "deadbeef",
            ));
        }

        let result = bridge.list_cached_agents().await.unwrap();
        let text = result_text(&result);

        // Each agent should appear as a bullet point.
        assert!(text.contains("• summarizer — Summarizes documents"));
        assert!(text.contains("• translator — Translates text between languages"));
        assert!(text.contains("• coder — Writes and reviews code"));

        // Should be 3 lines (one per agent).
        assert_eq!(text.lines().count(), 3);
    }

    // ── discover_agents formatting with multiple agents ──

    #[tokio::test]
    async fn discover_agents_numbered_list_format() {
        let transport = InMemoryTransport::new();

        let a = WakuA2ANode::new("alpha", "Agent A", vec!["a".into()], transport.clone());
        let b = WakuA2ANode::new("beta", "Agent B", vec!["b".into()], transport.clone());
        a.announce().await.unwrap();
        b.announce().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents().await.unwrap();
        let text = result_text(&result);

        assert!(text.contains("Found 2 agent(s)"));
        // Should contain numbered entries.
        assert!(text.contains("1. **"));
        assert!(text.contains("2. **"));
        assert!(text.contains("Capabilities: ["));
    }

    // ── discover_agents_presence ──

    #[tokio::test]
    async fn discover_agents_presence_finds_live_peers() {
        let transport = InMemoryTransport::new();

        // An agent announces presence on the shared transport.
        let agent = WakuA2ANode::new(
            "presence-agent",
            "Agent with presence",
            vec!["chat".into(), "search".into()],
            transport.clone(),
        );
        agent.announce_presence().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents_presence().await.unwrap();
        let text = result_text(&result);

        assert!(text.contains("Found 1 live agent(s) via presence"));
        assert!(text.contains("presence-agent"));
        assert!(text.contains("chat, search"));
        assert!(text.contains("Agent ID:"));
        assert!(text.contains("TTL:"));
    }

    #[tokio::test]
    async fn discover_agents_presence_no_peers() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let result = bridge.discover_agents_presence().await.unwrap();
        let text = result_text(&result);

        assert!(text.contains("No agents currently online via presence"));
    }

    #[tokio::test]
    async fn discover_agents_presence_multiple_peers() {
        let transport = InMemoryTransport::new();

        for (name, caps) in [
            ("agent-alpha", vec!["summarize"]),
            ("agent-beta", vec!["translate"]),
            ("agent-gamma", vec!["code", "review"]),
        ] {
            let node = WakuA2ANode::new(
                name,
                &format!("{name} agent"),
                caps.into_iter().map(String::from).collect(),
                transport.clone(),
            );
            node.announce_presence().await.unwrap();
        }

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents_presence().await.unwrap();
        let text = result_text(&result);

        assert!(text.contains("Found 3 live agent(s) via presence"));
        assert!(text.contains("agent-alpha"));
        assert!(text.contains("agent-beta"));
        assert!(text.contains("agent-gamma"));
    }

    #[tokio::test]
    async fn discover_agents_presence_numbered_format() {
        let transport = InMemoryTransport::new();

        let a = WakuA2ANode::new("first", "A", vec!["a".into()], transport.clone());
        let b = WakuA2ANode::new("second", "B", vec!["b".into()], transport.clone());
        a.announce_presence().await.unwrap();
        b.announce_presence().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents_presence().await.unwrap();
        let text = result_text(&result);

        // Should contain numbered entries with markdown bold.
        assert!(text.contains("1. **"));
        assert!(text.contains("2. **"));
        assert!(text.contains("Topic:"));
        assert!(text.contains("TTL:"));
    }

    #[tokio::test]
    async fn discover_agents_presence_excludes_self() {
        let transport = InMemoryTransport::new();

        // The bridge node itself announces presence — should be filtered out
        // by poll_presence's self-exclusion logic.
        let bridge = make_test_bridge(transport.clone());
        {
            let node = bridge.node.read().await;
            node.announce_presence().await.unwrap();
        }

        let result = bridge.discover_agents_presence().await.unwrap();
        let text = result_text(&result);
        assert!(text.contains("No agents currently online"));
    }

    #[tokio::test]
    async fn discover_agents_presence_ignores_unsigned() {
        use logos_messaging_a2a_core::{topics, A2AEnvelope, PresenceAnnouncement};

        let transport = InMemoryTransport::new();

        // Inject an unsigned presence announcement directly.
        let unsigned = PresenceAnnouncement {
            agent_id: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef00"
                .to_string(),
            name: "unsigned-agent".to_string(),
            capabilities: vec!["evil".into()],
            waku_topic: "/a2a/tasks/fake".to_string(),
            ttl_secs: 300,
            signature: None,
        };
        let envelope = A2AEnvelope::Presence(unsigned);
        let payload = serde_json::to_vec(&envelope).unwrap();
        transport.publish(topics::PRESENCE, &payload).await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge.discover_agents_presence().await.unwrap();
        let text = result_text(&result);

        // Unsigned announcement should be rejected.
        assert!(text.contains("No agents currently online"));
    }

    // ── get_agent_status ──

    #[tokio::test]
    async fn get_agent_status_online() {
        let transport = InMemoryTransport::new();

        let agent = WakuA2ANode::new(
            "status-agent",
            "Agent for status check",
            vec!["echo".into()],
            transport.clone(),
        );
        let agent_id = agent.pubkey().to_string();
        agent.announce_presence().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge
            .get_agent_status(Parameters(GetAgentStatusInput {
                agent_id: agent_id.clone(),
            }))
            .await
            .unwrap();
        let text = result_text(&result);

        assert!(text.contains("ONLINE"));
        assert!(text.contains("status-agent"));
        assert!(text.contains("echo"));
        assert!(text.contains("TTL:"));
        assert!(text.contains("last seen"));
    }

    #[tokio::test]
    async fn get_agent_status_offline() {
        let bridge = make_test_bridge(InMemoryTransport::new());
        let result = bridge
            .get_agent_status(Parameters(GetAgentStatusInput {
                agent_id: "nonexistent_agent_id_0000".to_string(),
            }))
            .await
            .unwrap();
        let text = result_text(&result);

        assert!(text.contains("OFFLINE or unknown"));
        assert!(text.contains("nonexistent_agent_id_0000"));
    }

    #[tokio::test]
    async fn get_agent_status_shows_capabilities() {
        let transport = InMemoryTransport::new();

        let agent = WakuA2ANode::new(
            "multi-cap",
            "Multi-capability agent",
            vec!["search".into(), "summarize".into(), "translate".into()],
            transport.clone(),
        );
        let agent_id = agent.pubkey().to_string();
        agent.announce_presence().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge
            .get_agent_status(Parameters(GetAgentStatusInput {
                agent_id: agent_id.clone(),
            }))
            .await
            .unwrap();
        let text = result_text(&result);

        assert!(text.contains("search, summarize, translate"));
    }

    #[tokio::test]
    async fn get_agent_status_shows_truncated_agent_id() {
        let transport = InMemoryTransport::new();

        let agent = WakuA2ANode::new(
            "truncated-id",
            "Agent",
            vec!["test".into()],
            transport.clone(),
        );
        let agent_id = agent.pubkey().to_string();
        agent.announce_presence().await.unwrap();

        let bridge = make_test_bridge(transport);
        let result = bridge
            .get_agent_status(Parameters(GetAgentStatusInput {
                agent_id: agent_id.clone(),
            }))
            .await
            .unwrap();
        let text = result_text(&result);

        // Agent ID should be truncated with "..."
        assert!(text.contains("..."));
        assert!(text.contains(&agent_id[..16]));
    }

    // ── GetAgentStatusInput deserialization ──

    #[test]
    fn get_agent_status_input_deserializes() {
        let json = r#"{"agent_id": "deadbeef01234567"}"#;
        let input: GetAgentStatusInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.agent_id, "deadbeef01234567");
    }

    #[test]
    fn get_agent_status_input_rejects_missing_field() {
        let json = r#"{}"#;
        assert!(serde_json::from_str::<GetAgentStatusInput>(json).is_err());
    }

    #[test]
    fn get_agent_status_input_accepts_extra_fields() {
        let json = r#"{"agent_id": "abc123", "extra": "ignored"}"#;
        let input: GetAgentStatusInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.agent_id, "abc123");
    }

    // ── format_peer_entry ──

    #[test]
    fn format_peer_entry_output() {
        use logos_messaging_a2a_node::presence::PeerInfo;

        let info = PeerInfo {
            name: "test-peer".to_string(),
            capabilities: vec!["echo".to_string(), "search".to_string()],
            waku_topic: "/a2a/tasks/abcdef".to_string(),
            ttl_secs: 300,
            last_seen: 1_700_000_000,
        };

        let output = format_peer_entry(1, "abcdef1234567890abcdef", &info);
        assert!(output.contains("1. **test-peer**"));
        assert!(output.contains("echo, search"));
        assert!(output.contains("Agent ID: abcdef1234567890..."));
        assert!(output.contains("Topic: /a2a/tasks/abcdef"));
        assert!(output.contains("TTL: 300s"));
    }

    #[test]
    fn format_peer_entry_short_agent_id() {
        use logos_messaging_a2a_node::presence::PeerInfo;

        let info = PeerInfo {
            name: "short-id".to_string(),
            capabilities: vec![],
            waku_topic: "/a2a/tasks/short".to_string(),
            ttl_secs: 60,
            last_seen: 0,
        };

        // Agent ID shorter than 16 chars should not panic.
        let output = format_peer_entry(1, "abc", &info);
        assert!(output.contains("abc..."));
    }

    // ── discover_agents description says legacy ──

    #[tokio::test]
    async fn discover_agents_description_mentions_legacy() {
        let bridge = make_test_bridge(InMemoryTransport::new());

        // Tool list should be available through the tool router.
        let tools = bridge.tool_router.list_all();
        let discover = tools.iter().find(|t| t.name == "discover_agents").unwrap();
        let desc = discover.description.as_deref().unwrap_or("");
        assert!(
            desc.contains("legacy"),
            "discover_agents description should mention 'legacy': {desc}"
        );
    }
}
