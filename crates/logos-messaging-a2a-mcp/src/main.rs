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

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use logos_messaging_a2a_core::{AgentCard, Part, TaskState};
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use logos_messaging_a2a_transport::Transport;
use serde::Deserialize;

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
struct SendToAgentInput {
    /// Name of the target agent (from discover_agents)
    agent_name: String,
    /// The message/task to send to the agent
    message: String,
}

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

/// Cached snapshot of discovered agents.
type AgentRegistry = Arc<RwLock<Vec<AgentCard>>>;

/// The MCP server that bridges to A2A over Waku.
struct LogosA2ABridge<T: Transport> {
    node: Arc<RwLock<WakuA2ANode<T>>>,
    agents: AgentRegistry,
    timeout_secs: u64,
    tool_router: ToolRouter<Self>,
}

// Manual Clone: T is behind Arc so we don't need T: Clone.
impl<T: Transport> Clone for LogosA2ABridge<T> {
    fn clone(&self) -> Self {
        Self {
            node: self.node.clone(),
            agents: self.agents.clone(),
            timeout_secs: self.timeout_secs,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl<T: Transport> LogosA2ABridge<T> {
    /// Create a bridge wrapping an existing node.
    fn from_node(node: WakuA2ANode<T>, timeout_secs: u64) -> Self {
        Self {
            node: Arc::new(RwLock::new(node)),
            agents: Arc::new(RwLock::new(Vec::new())),
            timeout_secs,
            tool_router: Self::tool_router(),
        }
    }

    /// Discover all agents currently advertising on the Waku network.
    #[tool(
        description = "Discover agents on the Logos messaging network. Returns a list of agent names, descriptions, and capabilities. Call this first to see what agents are available."
    )]
    async fn discover_agents(&self) -> Result<CallToolResult, McpError> {
        let node = self.node.read().await;
        let cards = node.discover().await.map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: format!("Discovery failed: {e}").into(),
            data: None,
        })?;

        let mut agents = self.agents.write().await;
        *agents = cards.clone();

        if cards.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No agents found on the network. Make sure nwaku is running and agents are announced.",
            )]));
        }

        let summary: Vec<String> = cards
            .iter()
            .enumerate()
            .map(|(i, c)| {
                format!(
                    "{}. **{}** (v{}) — {}\n   Capabilities: [{}]\n   Public key: {}...",
                    i + 1,
                    c.name,
                    c.version,
                    c.description,
                    c.capabilities.join(", "),
                    &c.public_key[..16]
                )
            })
            .collect();

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Found {} agent(s):\n\n{}",
            cards.len(),
            summary.join("\n\n")
        ))]))
    }

    /// Send a task/message to a specific agent by name and wait for a response.
    #[tool(
        description = "Send a message to a Logos agent by name. The agent will process it and return a response. Call discover_agents first to see available agents."
    )]
    async fn send_to_agent(
        &self,
        Parameters(SendToAgentInput {
            agent_name,
            message,
        }): Parameters<SendToAgentInput>,
    ) -> Result<CallToolResult, McpError> {
        let agents = self.agents.read().await;
        let card = agents.iter().find(|c| c.name == agent_name).cloned();
        drop(agents);

        let card = card.ok_or_else(|| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: format!(
                "Agent '{agent_name}' not found. Call discover_agents first to refresh the list."
            )
            .into(),
            data: None,
        })?;

        let node = self.node.read().await;
        let task = node
            .send_text(&card.public_key, &message)
            .await
            .map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: format!("Failed to send task: {e}").into(),
                data: None,
            })?;

        let deadline =
            tokio::time::Instant::now() + tokio::time::Duration::from_secs(self.timeout_secs);
        let task_id = task.id.clone();

        loop {
            if tokio::time::Instant::now() > deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timeout waiting for response from '{agent_name}' (task {}). The agent may still be processing.",
                    task_id
                ))]));
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            let tasks = node.poll_tasks().await.map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: format!("Poll failed: {e}").into(),
                data: None,
            })?;

            if let Some(response) = tasks.iter().find(|t| t.id == task_id) {
                match response.state {
                    TaskState::Completed => {
                        let text = response
                            .result
                            .as_ref()
                            .map(|m| {
                                m.parts
                                    .iter()
                                    .map(|p| match p {
                                        Part::Text { text } => text.as_str(),
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_else(|| "(no result body)".into());

                        return Ok(CallToolResult::success(vec![Content::text(format!(
                            "Response from '{agent_name}':\n\n{text}"
                        ))]));
                    }
                    TaskState::Failed => {
                        return Ok(CallToolResult::error(vec![Content::text(format!(
                            "Agent '{agent_name}' reported task failed (task {})",
                            task_id
                        ))]));
                    }
                    _ => continue,
                }
            }
        }
    }

    /// List the currently cached agents (no network call).
    #[tool(
        description = "List agents from the last discovery (cached). Use discover_agents to refresh."
    )]
    async fn list_cached_agents(&self) -> Result<CallToolResult, McpError> {
        let agents = self.agents.read().await;
        if agents.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No cached agents. Call discover_agents first.",
            )]));
        }

        let names: Vec<String> = agents
            .iter()
            .map(|c| format!("• {} — {}", c.name, c.description))
            .collect();

        Ok(CallToolResult::success(vec![Content::text(
            names.join("\n"),
        )]))
    }
}

impl LogosA2ABridge<LogosMessagingTransport> {
    fn new(waku_url: &str, timeout_secs: u64) -> Self {
        let transport = LogosMessagingTransport::new(waku_url);
        let node = WakuA2ANode::new(
            "mcp-bridge",
            "MCP bridge — proxies tool calls to Logos A2A agents",
            vec!["mcp-bridge".into()],
            transport,
        );
        Self::from_node(node, timeout_secs)
    }
}

#[tool_handler]
impl<T: Transport> ServerHandler for LogosA2ABridge<T> {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Logos Messaging A2A Bridge — discover and communicate with agents on the \
                 Logos/Waku decentralized network. Call discover_agents first, then send_to_agent \
                 to interact with specific agents."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
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

    let bridge = LogosA2ABridge::new(&cli.waku_url, cli.timeout);

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
    use logos_messaging_a2a_transport::memory::InMemoryTransport;

    /// Extract text from the first content element of a CallToolResult.
    fn result_text(result: &CallToolResult) -> &str {
        match &result.content[0].raw {
            RawContent::Text(t) => t.text.as_str(),
            _ => panic!("expected text content"),
        }
    }

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
}
