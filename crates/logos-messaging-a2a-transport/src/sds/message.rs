//! SDS message types: Content, Sync, and Ephemeral.
//!
//! Based on the Logos Messaging SDS specification.
//! Reference: logos-messaging/logos-delivery-js MessageChannel

use sha2::{Digest, Sha256};
use std::fmt;

/// Unique message identifier (hex-encoded SHA-256 of payload).
pub type MessageId = String;
/// Channel identifier.
pub type ChannelId = String;
/// Participant identifier.
pub type ParticipantId = String;

/// An entry in the causal history, recording a message and its lamport timestamp.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    pub message_id: MessageId,
    pub lamport_timestamp: u64,
    /// Optional retrieval hint (e.g. store node address) to locate this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_hint: Option<Vec<u8>>,
}

/// The three SDS message types.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum SdsMessage {
    /// Content message: has payload, lamport timestamp, and causal history.
    Content(ContentMessage),
    /// Sync message: no payload, carries bloom filter + causal history for consistency.
    Sync(SyncMessage),
    /// Ephemeral message: has payload but no lamport timestamp (not causally ordered).
    Ephemeral(EphemeralMessage),
}

impl SdsMessage {
    pub fn message_id(&self) -> &str {
        match self {
            SdsMessage::Content(m) => &m.message_id,
            SdsMessage::Sync(m) => &m.message_id,
            SdsMessage::Ephemeral(m) => &m.message_id,
        }
    }

    pub fn channel_id(&self) -> &str {
        match self {
            SdsMessage::Content(m) => &m.channel_id,
            SdsMessage::Sync(m) => &m.channel_id,
            SdsMessage::Ephemeral(m) => &m.channel_id,
        }
    }

    pub fn sender_id(&self) -> &str {
        match self {
            SdsMessage::Content(m) => &m.sender_id,
            SdsMessage::Sync(m) => &m.sender_id,
            SdsMessage::Ephemeral(m) => &m.sender_id,
        }
    }

    pub fn causal_history(&self) -> &[HistoryEntry] {
        match self {
            SdsMessage::Content(m) => &m.causal_history,
            SdsMessage::Sync(m) => &m.causal_history,
            SdsMessage::Ephemeral(m) => &m.causal_history,
        }
    }

    pub fn bloom_filter_bytes(&self) -> Option<&[u8]> {
        match self {
            SdsMessage::Content(m) => m.bloom_filter.as_deref(),
            SdsMessage::Sync(m) => m.bloom_filter.as_deref(),
            SdsMessage::Ephemeral(m) => m.bloom_filter.as_deref(),
        }
    }

    pub fn repair_requests(&self) -> &[HistoryEntry] {
        match self {
            SdsMessage::Content(m) => &m.repair_request,
            SdsMessage::Sync(m) => &m.repair_request,
            SdsMessage::Ephemeral(m) => &m.repair_request,
        }
    }
}

/// Content message — carries actual payload with causal ordering.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContentMessage {
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    pub sender_id: ParticipantId,
    pub lamport_timestamp: u64,
    pub causal_history: Vec<HistoryEntry>,
    pub bloom_filter: Option<Vec<u8>>,
    pub content: Vec<u8>,
    #[serde(default)]
    pub repair_request: Vec<HistoryEntry>,
    #[serde(skip)]
    pub retrieval_hint: Option<Vec<u8>>,
}

/// Sync message — no payload, used for consistency protocol (bloom filter exchange).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyncMessage {
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    pub sender_id: ParticipantId,
    pub lamport_timestamp: u64,
    pub causal_history: Vec<HistoryEntry>,
    pub bloom_filter: Option<Vec<u8>>,
    #[serde(default)]
    pub repair_request: Vec<HistoryEntry>,
}

/// Ephemeral message — has payload but no causal ordering (fire-and-forget).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EphemeralMessage {
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    pub sender_id: ParticipantId,
    pub causal_history: Vec<HistoryEntry>,
    pub bloom_filter: Option<Vec<u8>>,
    pub content: Vec<u8>,
    #[serde(default)]
    pub repair_request: Vec<HistoryEntry>,
}

/// Compute a message ID from payload bytes (SHA-256, hex-encoded).
pub fn compute_message_id(payload: &[u8]) -> MessageId {
    let hash = Sha256::digest(payload);
    hex::encode(hash)
}

impl fmt::Display for SdsMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdsMessage::Content(m) => {
                write!(
                    f,
                    "Content(id={}, ts={})",
                    m.message_id, m.lamport_timestamp
                )
            }
            SdsMessage::Sync(m) => {
                write!(f, "Sync(id={}, ts={})", m.message_id, m.lamport_timestamp)
            }
            SdsMessage::Ephemeral(m) => write!(f, "Ephemeral(id={})", m.message_id),
        }
    }
}
