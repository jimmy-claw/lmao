//! Multi-turn conversation sessions between agents.

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// A multi-turn conversation session between two agents.
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    /// Unique session identifier (UUID v4).
    pub id: String,
    /// Public key of the remote peer in this session.
    pub peer: String,
    /// Task IDs exchanged within this session, in chronological order.
    pub task_ids: Vec<String>,
    /// Unix timestamp (seconds) when this session was created.
    pub created_at: u64,
}

impl Session {
    pub(crate) fn new(peer: &str) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id,
            peer: peer.to_string(),
            task_ids: Vec::new(),
            created_at,
        }
    }
}
