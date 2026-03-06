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

/// Convenience constructor for [`ContentMessage`].
///
/// # Example
///
/// ```
/// use logos_messaging_a2a_transport::sds::message::ContentMessage;
///
/// let msg = ContentMessage::new("chan-1", "alice", 1, b"hello");
/// assert_eq!(msg.sender_id, "alice");
/// assert_eq!(msg.content, b"hello");
/// ```
impl ContentMessage {
    /// Construct a content message, computing the message ID from `content`.
    pub fn new(channel_id: &str, sender_id: &str, lamport_timestamp: u64, content: &[u8]) -> Self {
        Self {
            message_id: compute_message_id(content),
            channel_id: channel_id.to_string(),
            sender_id: sender_id.to_string(),
            lamport_timestamp,
            causal_history: Vec::new(),
            bloom_filter: None,
            content: content.to_vec(),
            repair_request: Vec::new(),
            retrieval_hint: None,
        }
    }
}

/// Convenience constructor for [`EphemeralMessage`].
impl EphemeralMessage {
    /// Construct an ephemeral message, computing the message ID from `content`.
    pub fn new(channel_id: &str, sender_id: &str, content: &[u8]) -> Self {
        Self {
            message_id: compute_message_id(content),
            channel_id: channel_id.to_string(),
            sender_id: sender_id.to_string(),
            causal_history: Vec::new(),
            bloom_filter: None,
            content: content.to_vec(),
            repair_request: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_message_id_deterministic() {
        let id1 = compute_message_id(b"hello");
        let id2 = compute_message_id(b"hello");
        assert_eq!(id1, id2);
        // SHA-256 of "hello" is well-known
        assert_eq!(
            id1,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_compute_message_id_different_inputs() {
        let id1 = compute_message_id(b"hello");
        let id2 = compute_message_id(b"world");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_content_message_new() {
        let msg = ContentMessage::new("chan-1", "alice", 42, b"payload");
        assert_eq!(msg.channel_id, "chan-1");
        assert_eq!(msg.sender_id, "alice");
        assert_eq!(msg.lamport_timestamp, 42);
        assert_eq!(msg.content, b"payload");
        assert_eq!(msg.message_id, compute_message_id(b"payload"));
        assert!(msg.causal_history.is_empty());
        assert!(msg.bloom_filter.is_none());
        assert!(msg.repair_request.is_empty());
    }

    #[test]
    fn test_ephemeral_message_new() {
        let msg = EphemeralMessage::new("chan-1", "bob", b"fire-and-forget");
        assert_eq!(msg.channel_id, "chan-1");
        assert_eq!(msg.sender_id, "bob");
        assert_eq!(msg.content, b"fire-and-forget");
        assert_eq!(msg.message_id, compute_message_id(b"fire-and-forget"));
        assert!(msg.causal_history.is_empty());
        assert!(msg.bloom_filter.is_none());
    }

    #[test]
    fn test_content_message_serialization_roundtrip() {
        let msg = ContentMessage::new("chan-1", "alice", 10, b"test data");
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ContentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.message_id, deserialized.message_id);
        assert_eq!(msg.content, deserialized.content);
        assert_eq!(msg.lamport_timestamp, deserialized.lamport_timestamp);
    }

    #[test]
    fn test_sync_message_serialization_roundtrip() {
        let msg = SyncMessage {
            message_id: "sync-id".to_string(),
            channel_id: "chan-1".to_string(),
            sender_id: "alice".to_string(),
            lamport_timestamp: 5,
            causal_history: vec![HistoryEntry {
                message_id: "dep-1".to_string(),
                lamport_timestamp: 3,
                retrieval_hint: None,
            }],
            bloom_filter: Some(vec![0xFF, 0x00]),
            repair_request: Vec::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: SyncMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.message_id, deserialized.message_id);
        assert_eq!(msg.causal_history.len(), 1);
        assert_eq!(deserialized.causal_history[0].message_id, "dep-1");
    }

    #[test]
    fn test_ephemeral_message_serialization_roundtrip() {
        let msg = EphemeralMessage::new("chan-1", "bob", b"ephemeral payload");
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: EphemeralMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.message_id, deserialized.message_id);
        assert_eq!(msg.content, deserialized.content);
    }

    #[test]
    fn test_sds_message_enum_tagged_serialization() {
        let content = SdsMessage::Content(ContentMessage::new("c", "a", 1, b"x"));
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains("\"type\":\"Content\""));

        let ephemeral = SdsMessage::Ephemeral(EphemeralMessage::new("c", "b", b"y"));
        let json = serde_json::to_string(&ephemeral).unwrap();
        assert!(json.contains("\"type\":\"Ephemeral\""));

        let sync = SdsMessage::Sync(SyncMessage {
            message_id: "s1".to_string(),
            channel_id: "c".to_string(),
            sender_id: "a".to_string(),
            lamport_timestamp: 1,
            causal_history: Vec::new(),
            bloom_filter: None,
            repair_request: Vec::new(),
        });
        let json = serde_json::to_string(&sync).unwrap();
        assert!(json.contains("\"type\":\"Sync\""));
    }

    #[test]
    fn test_sds_message_accessors() {
        let content = SdsMessage::Content(ContentMessage {
            message_id: "msg-1".to_string(),
            channel_id: "chan-1".to_string(),
            sender_id: "alice".to_string(),
            lamport_timestamp: 10,
            causal_history: vec![HistoryEntry {
                message_id: "dep-0".to_string(),
                lamport_timestamp: 9,
                retrieval_hint: Some(vec![1, 2, 3]),
            }],
            bloom_filter: Some(vec![0xAB]),
            content: b"hello".to_vec(),
            repair_request: vec![HistoryEntry {
                message_id: "repair-1".to_string(),
                lamport_timestamp: 8,
                retrieval_hint: None,
            }],
            retrieval_hint: None,
        });

        assert_eq!(content.message_id(), "msg-1");
        assert_eq!(content.channel_id(), "chan-1");
        assert_eq!(content.sender_id(), "alice");
        assert_eq!(content.causal_history().len(), 1);
        assert_eq!(content.bloom_filter_bytes(), Some(&[0xAB][..]));
        assert_eq!(content.repair_requests().len(), 1);
        assert_eq!(content.repair_requests()[0].message_id, "repair-1");
    }

    #[test]
    fn test_sds_message_display() {
        let content = SdsMessage::Content(ContentMessage::new("c", "a", 42, b"x"));
        let display = format!("{}", content);
        assert!(display.contains("Content"));
        assert!(display.contains("42"));

        let sync = SdsMessage::Sync(SyncMessage {
            message_id: "s1".to_string(),
            channel_id: "c".to_string(),
            sender_id: "a".to_string(),
            lamport_timestamp: 7,
            causal_history: Vec::new(),
            bloom_filter: None,
            repair_request: Vec::new(),
        });
        assert!(format!("{}", sync).contains("Sync"));
        assert!(format!("{}", sync).contains("7"));

        let eph = SdsMessage::Ephemeral(EphemeralMessage::new("c", "b", b"y"));
        assert!(format!("{}", eph).contains("Ephemeral"));
    }

    #[test]
    fn test_history_entry_with_retrieval_hint() {
        let entry = HistoryEntry {
            message_id: "msg-1".to_string(),
            lamport_timestamp: 5,
            retrieval_hint: Some(vec![10, 20, 30]),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("retrieval_hint"));

        let entry_no_hint = HistoryEntry {
            message_id: "msg-2".to_string(),
            lamport_timestamp: 6,
            retrieval_hint: None,
        };
        let json = serde_json::to_string(&entry_no_hint).unwrap();
        // retrieval_hint should be absent when None (skip_serializing_if)
        assert!(!json.contains("retrieval_hint"));
    }

    #[test]
    fn test_content_message_default_repair_request() {
        // Deserializing without repair_request field should default to empty vec
        let json = r#"{
            "message_id": "m1",
            "channel_id": "c1",
            "sender_id": "s1",
            "lamport_timestamp": 1,
            "causal_history": [],
            "bloom_filter": null,
            "content": [104, 101, 108, 108, 111]
        }"#;
        let msg: ContentMessage = serde_json::from_str(json).unwrap();
        assert!(msg.repair_request.is_empty());
        assert_eq!(msg.content, b"hello");
    }

    #[test]
    fn test_ephemeral_no_bloom_no_causal_history() {
        let msg = SdsMessage::Ephemeral(EphemeralMessage::new("c", "a", b"data"));
        assert!(msg.bloom_filter_bytes().is_none());
        assert!(msg.causal_history().is_empty());
    }
}
