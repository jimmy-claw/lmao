//! MCP Bridge Server for Logos Messaging A2A
//!
//! Exposes discovered Waku A2A agents as MCP tools.
//! Each agent on the network becomes a callable tool in Claude Desktop, Cursor, etc.
//!
//! Architecture:
//!   MCP Host (Claude) → stdio → logos-messaging-a2a-mcp → Waku → Agent Fleet

use std::sync::Arc;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use tokio::sync::RwLock;

use logos_messaging_a2a_core::{Part, TaskState};
use logos_messaging_a2a_node::presence::PeerInfo;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::Transport;
use serde::Deserialize;

pub use logos_messaging_a2a_core::AgentCard;

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
pub struct SendToAgentInput {
    /// Name of the target agent (from discover_agents)
    pub agent_name: String,
    /// The message/task to send to the agent
    pub message: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
pub struct GetAgentStatusInput {
    /// Agent ID (public key hex) to check presence status for
    pub agent_id: String,
}

/// Cached snapshot of discovered agents.
pub type AgentRegistry = Arc<RwLock<Vec<AgentCard>>>;

/// The MCP server that bridges to A2A over Waku.
pub struct LogosA2ABridge<T: Transport> {
    pub node: Arc<RwLock<WakuA2ANode<T>>>,
    pub agents: AgentRegistry,
    pub timeout_secs: u64,
    pub tool_router: ToolRouter<Self>,
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
    pub fn from_node(node: WakuA2ANode<T>, timeout_secs: u64) -> Self {
        Self {
            node: Arc::new(RwLock::new(node)),
            agents: Arc::new(RwLock::new(Vec::new())),
            timeout_secs,
            tool_router: Self::tool_router(),
        }
    }

    /// Discover agents via legacy broadcast discovery (subscribes to the discovery topic and
    /// drains historical announcements). For real-time presence-based discovery, use
    /// `discover_agents_presence` instead.
    #[tool(
        description = "Discover agents via legacy broadcast discovery (drains the discovery topic). Returns agent names, descriptions, and capabilities. For real-time presence-aware discovery with online status, prefer discover_agents_presence instead."
    )]
    pub async fn discover_agents(&self) -> Result<CallToolResult, McpError> {
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
    pub async fn send_to_agent(
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

            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

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
    pub async fn list_cached_agents(&self) -> Result<CallToolResult, McpError> {
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

    /// Discover agents via real-time presence broadcasts.
    ///
    /// Polls the presence topic for signed announcements and returns all
    /// agents that are currently online (within their TTL window).
    #[tool(
        description = "Discover agents via real-time presence broadcasts. Polls the Waku presence topic for signed announcements and returns agents that are currently online (within their TTL). More reliable than legacy discover_agents for checking who is actually live right now."
    )]
    pub async fn discover_agents_presence(&self) -> Result<CallToolResult, McpError> {
        let node = self.node.read().await;
        node.poll_presence().await.map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: format!("Presence poll failed: {e}").into(),
            data: None,
        })?;

        let live_peers = node.peers().all_live();

        if live_peers.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No agents currently online via presence. Agents may not have announced presence yet, or their TTL may have expired.",
            )]));
        }

        let summary: Vec<String> = live_peers
            .iter()
            .enumerate()
            .map(|(i, (agent_id, info))| format_peer_entry(i + 1, agent_id, info))
            .collect();

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Found {} live agent(s) via presence:\n\n{}",
            live_peers.len(),
            summary.join("\n\n")
        ))]))
    }

    /// Check if a specific agent is currently online via presence.
    #[tool(
        description = "Check if a specific agent is currently online by its agent ID (public key hex). Polls for fresh presence data and returns the agent's status, capabilities, and TTL info."
    )]
    pub async fn get_agent_status(
        &self,
        Parameters(GetAgentStatusInput { agent_id }): Parameters<GetAgentStatusInput>,
    ) -> Result<CallToolResult, McpError> {
        let node = self.node.read().await;
        node.poll_presence().await.map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: format!("Presence poll failed: {e}").into(),
            data: None,
        })?;

        match node.peers().get(&agent_id) {
            Some(info) => {
                let elapsed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .saturating_sub(info.last_seen);

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Agent **{}** is ONLINE\n\
                     • Agent ID: {}...\n\
                     • Capabilities: [{}]\n\
                     • Waku topic: {}\n\
                     • TTL: {}s (last seen {}s ago)",
                    info.name,
                    &agent_id[..16.min(agent_id.len())],
                    info.capabilities.join(", "),
                    info.waku_topic,
                    info.ttl_secs,
                    elapsed,
                ))]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Agent '{agent_id}' is OFFLINE or unknown. The agent may not have announced presence, or its TTL has expired."
            ))])),
        }
    }
}

/// Format a single peer entry for display.
pub fn format_peer_entry(index: usize, agent_id: &str, info: &PeerInfo) -> String {
    format!(
        "{}. **{}** — [{}]\n   Agent ID: {}...\n   Topic: {}\n   TTL: {}s",
        index,
        info.name,
        info.capabilities.join(", "),
        &agent_id[..16.min(agent_id.len())],
        info.waku_topic,
        info.ttl_secs,
    )
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

/// Extract text from the first content element of a [`CallToolResult`].
pub fn result_text(result: &CallToolResult) -> &str {
    match &result.content[0].raw {
        RawContent::Text(t) => t.text.as_str(),
        _ => panic!("expected text content"),
    }
}
