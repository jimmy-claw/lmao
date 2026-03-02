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

use serde::Deserialize;
use logos_messaging_a2a_core::{AgentCard, Part, TaskState};
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
struct SendToAgentInput {
    /// Name of the target agent (from discover_agents)
    agent_name: String,
    /// The message/task to send to the agent
    message: String,
}

#[derive(Parser)]
#[command(name = "logos-messaging-a2a-mcp", about = "MCP bridge for Logos A2A agents")]
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
#[derive(Clone)]
struct LogosA2ABridge {
    node: Arc<RwLock<WakuA2ANode<LogosMessagingTransport>>>,
    agents: AgentRegistry,
    timeout_secs: u64,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl LogosA2ABridge {
    fn new(waku_url: &str, timeout_secs: u64) -> Self {
        let transport = LogosMessagingTransport::new(waku_url);
        let node = WakuA2ANode::new(
            "mcp-bridge",
            "MCP bridge — proxies tool calls to Logos A2A agents",
            vec!["mcp-bridge".into()],
            transport,
        );

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

#[tool_handler]
impl ServerHandler for LogosA2ABridge {
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
