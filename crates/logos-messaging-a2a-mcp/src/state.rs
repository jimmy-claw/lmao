use std::sync::Arc;

use logos_messaging_a2a_core::AgentCard;
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
pub(crate) struct SendToAgentInput {
    /// Name of the target agent (from discover_agents)
    pub agent_name: String,
    /// The message/task to send to the agent
    pub message: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
pub(crate) struct GetAgentStatusInput {
    /// Agent ID (public key hex) to check presence status for
    pub agent_id: String,
}

/// Cached snapshot of discovered agents.
pub(crate) type AgentRegistry = Arc<RwLock<Vec<AgentCard>>>;
