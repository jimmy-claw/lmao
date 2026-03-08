use logos_messaging_a2a_crypto::{EncryptedPayload, IntroBundle};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Agent identity and capability advertisement.
/// Equivalent to A2A's AgentCard — broadcast on the discovery topic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub version: String,
    pub capabilities: Vec<String>,
    /// secp256k1 compressed public key as hex string — agent identity
    pub public_key: String,
    /// X25519 intro bundle for encrypted sessions (None = no encryption)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro_bundle: Option<IntroBundle>,
}

/// Task lifecycle states (A2A spec).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Failed,
    Cancelled,
}

/// A message part. Text-only for v0.1; extensible to images, files, etc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
}

/// A message within a task (user or agent turn).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: String,
    pub parts: Vec<Part>,
}

/// An A2A task: the unit of work exchanged between agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub from: String,
    pub to: String,
    pub state: TaskState,
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Message>,
    /// Session ID for multi-turn conversations. Tasks with the same
    /// session_id belong to the same conversation thread.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    /// CID of a large payload offloaded to Logos Storage (Codex).
    /// When present, the actual data can be fetched via a `StorageBackend`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub payload_cid: Option<String>,
    /// Transaction hash proving payment was made (x402-style payment flow).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub payment_tx: Option<String>,
    /// Amount paid (in token units).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub payment_amount: Option<u64>,
}

/// Presence announcement broadcast on the well-known presence topic.
///
/// Agents periodically publish these so peers can build a live map of
/// available agents and their capabilities without querying a registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresenceAnnouncement {
    /// Agent public key (secp256k1 compressed hex) — unique identity.
    pub agent_id: String,
    /// Human-readable agent name.
    pub name: String,
    /// Capabilities this agent offers (e.g. `["summarize", "translate"]`).
    pub capabilities: Vec<String>,
    /// Waku content topic where this agent receives tasks.
    pub waku_topic: String,
    /// How long (seconds) this announcement is valid. Peers should evict
    /// entries older than `ttl_secs` without a refresh.
    pub ttl_secs: u64,
    /// Optional secp256k1 signature over the canonical JSON of the other
    /// fields, proving the announcement comes from the claimed `agent_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

/// Wire envelope for all messages on Waku topics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum A2AEnvelope {
    AgentCard(AgentCard),
    Task(Task),
    Ack {
        message_id: String,
    },
    EncryptedTask {
        encrypted: EncryptedPayload,
        sender_pubkey: String,
    },
    Presence(PresenceAnnouncement),
}

impl Task {
    pub fn new(from: &str, to: &str, text: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from: from.to_string(),
            to: to.to_string(),
            state: TaskState::Submitted,
            message: Message {
                role: "user".to_string(),
                parts: vec![Part::Text {
                    text: text.to_string(),
                }],
            },
            result: None,
            payload_cid: None,
            session_id: None,
            payment_tx: None,
            payment_amount: None,
        }
    }

    /// Create a task within a session.
    pub fn new_in_session(from: &str, to: &str, text: &str, session_id: &str) -> Self {
        let mut task = Self::new(from, to, text);
        task.session_id = Some(session_id.to_string());
        task
    }

    pub fn respond(&self, text: &str) -> Self {
        Self {
            id: self.id.clone(),
            from: self.to.clone(),
            to: self.from.clone(),
            state: TaskState::Completed,
            message: self.message.clone(),
            result: Some(Message {
                role: "agent".to_string(),
                parts: vec![Part::Text {
                    text: text.to_string(),
                }],
            }),
            payload_cid: None,
            session_id: self.session_id.clone(),
            payment_tx: None,
            payment_amount: None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        self.message
            .parts
            .iter()
            .map(|p| match p {
                Part::Text { text } => text.as_str(),
            })
            .next()
    }

    pub fn result_text(&self) -> Option<&str> {
        self.result.as_ref().and_then(|m| {
            m.parts
                .iter()
                .map(|p| match p {
                    Part::Text { text } => text.as_str(),
                })
                .next()
        })
    }
}

/// Waku content topic helpers.
pub mod topics {
    pub const DISCOVERY: &str = "/waku-a2a/1/discovery/proto";

    /// Well-known topic for presence announcements.
    /// All agents subscribe on startup to discover live peers.
    pub const PRESENCE: &str = "/lmao/1/presence/proto";

    pub fn task_topic(recipient_pubkey: &str) -> String {
        format!("/waku-a2a/1/task/{}/proto", recipient_pubkey)
    }

    pub fn ack_topic(message_id: &str) -> String {
        format!("/waku-a2a/1/ack/{}/proto", message_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new("02aabb", "03ccdd", "Hello agent");
        assert_eq!(task.from, "02aabb");
        assert_eq!(task.to, "03ccdd");
        assert_eq!(task.state, TaskState::Submitted);
        assert_eq!(task.text(), Some("Hello agent"));
        assert!(task.result.is_none());
        assert!(!task.id.is_empty());
    }

    #[test]
    fn test_task_respond() {
        let task = Task::new("02aabb", "03ccdd", "Hello");
        let response = task.respond("Echo: Hello");
        assert_eq!(response.id, task.id);
        assert_eq!(response.from, "03ccdd");
        assert_eq!(response.to, "02aabb");
        assert_eq!(response.state, TaskState::Completed);
        assert_eq!(response.result_text(), Some("Echo: Hello"));
    }

    #[test]
    fn test_agent_card_serialization() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "Echoes messages".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02abcdef".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, deserialized);
        // intro_bundle should be absent when None
        assert!(!json.contains("intro_bundle"));
    }

    #[test]
    fn test_agent_card_with_intro_bundle() {
        let card = AgentCard {
            name: "echo".to_string(),
            description: "Echoes messages".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02abcdef".to_string(),
            intro_bundle: Some(IntroBundle::new("aabbccdd")),
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, deserialized);
        assert!(json.contains("intro_bundle"));
    }

    #[test]
    fn test_envelope_serialization() {
        let task = Task::new("02aa", "03bb", "test");
        let envelope = A2AEnvelope::Task(task.clone());
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, deserialized);

        let ack = A2AEnvelope::Ack {
            message_id: "abc-123".to_string(),
        };
        let json = serde_json::to_string(&ack).unwrap();
        assert!(json.contains("ack"));
        let deserialized: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(ack, deserialized);
    }

    #[test]
    fn test_encrypted_task_envelope_serialization() {
        let envelope = A2AEnvelope::EncryptedTask {
            encrypted: EncryptedPayload {
                nonce: "dGVzdG5vbmNl".to_string(),
                ciphertext: "Y2lwaGVydGV4dA==".to_string(),
            },
            sender_pubkey: "aabbccdd".to_string(),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, deserialized);
        assert!(json.contains("encrypted_task"));
    }

    #[test]
    fn test_task_state_serialization() {
        let states = vec![
            (TaskState::Submitted, "\"submitted\""),
            (TaskState::Working, "\"working\""),
            (TaskState::InputRequired, "\"input_required\""),
            (TaskState::Completed, "\"completed\""),
            (TaskState::Failed, "\"failed\""),
            (TaskState::Cancelled, "\"cancelled\""),
        ];
        for (state, expected) in states {
            assert_eq!(serde_json::to_string(&state).unwrap(), expected);
        }
    }

    #[test]
    fn test_topics() {
        assert_eq!(topics::DISCOVERY, "/waku-a2a/1/discovery/proto");
        assert_eq!(
            topics::task_topic("02abcdef"),
            "/waku-a2a/1/task/02abcdef/proto"
        );
        assert_eq!(
            topics::ack_topic("msg-123"),
            "/waku-a2a/1/ack/msg-123/proto"
        );
    }

    #[test]
    fn test_backward_compat_agent_card_without_intro_bundle() {
        // JSON without intro_bundle field should deserialize fine (defaults to None)
        let json = r#"{"name":"echo","description":"Echoes","version":"0.1.0","capabilities":["text"],"public_key":"02abcdef"}"#;
        let card: AgentCard = serde_json::from_str(json).unwrap();
        assert_eq!(card.name, "echo");
        assert!(card.intro_bundle.is_none());
    }

    #[test]
    fn test_presence_announcement_serialization() {
        let ann = PresenceAnnouncement {
            agent_id: "02abcdef".to_string(),
            name: "echo".to_string(),
            capabilities: vec!["text".to_string(), "summarize".to_string()],
            waku_topic: "/waku-a2a/1/task/02abcdef/proto".to_string(),
            ttl_secs: 300,
            signature: None,
        };
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: PresenceAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(ann, deserialized);
        assert!(!json.contains("signature"));
    }

    #[test]
    fn test_presence_envelope_serialization() {
        let ann = PresenceAnnouncement {
            agent_id: "02abcdef".to_string(),
            name: "echo".to_string(),
            capabilities: vec!["text".to_string()],
            waku_topic: "/waku-a2a/1/task/02abcdef/proto".to_string(),
            ttl_secs: 300,
            signature: Some(vec![0xab, 0xcd]),
        };
        let envelope = A2AEnvelope::Presence(ann.clone());
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("presence"));
        let deserialized: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, deserialized);
    }

    #[test]
    fn test_presence_topic() {
        assert_eq!(topics::PRESENCE, "/lmao/1/presence/proto");
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;

    #[test]
    fn test_new_in_session() {
        let task = Task::new_in_session("02aa", "03bb", "hello", "session-42");
        assert_eq!(task.from, "02aa");
        assert_eq!(task.to, "03bb");
        assert_eq!(task.text(), Some("hello"));
        assert_eq!(task.session_id, Some("session-42".to_string()));
        assert_eq!(task.state, TaskState::Submitted);
    }

    #[test]
    fn test_respond_preserves_session_id() {
        let task = Task::new_in_session("02aa", "03bb", "question", "sess-1");
        let response = task.respond("answer");
        assert_eq!(response.session_id, Some("sess-1".to_string()));
        assert_eq!(response.from, "03bb");
        assert_eq!(response.to, "02aa");
    }

    #[test]
    fn test_result_text_none_when_no_result() {
        let task = Task::new("02aa", "03bb", "hello");
        assert!(task.result_text().is_none());
    }

    #[test]
    fn test_task_optional_fields_absent_in_json() {
        let task = Task::new("02aa", "03bb", "minimal");
        let json = serde_json::to_string(&task).unwrap();
        assert!(!json.contains("session_id"));
        assert!(!json.contains("payload_cid"));
        assert!(!json.contains("payment_tx"));
        assert!(!json.contains("payment_amount"));
    }

    #[test]
    fn test_task_optional_fields_roundtrip() {
        let mut task = Task::new("02aa", "03bb", "full");
        task.session_id = Some("sess-1".to_string());
        task.payload_cid = Some("zQm123".to_string());
        task.payment_tx = Some("0xdeadbeef".to_string());
        task.payment_amount = Some(42);

        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_id, Some("sess-1".to_string()));
        assert_eq!(deserialized.payload_cid, Some("zQm123".to_string()));
        assert_eq!(deserialized.payment_tx, Some("0xdeadbeef".to_string()));
        assert_eq!(deserialized.payment_amount, Some(42));
    }

    #[test]
    fn test_backward_compat_task_without_optional_fields() {
        let json = r#"{"id":"abc","from":"02aa","to":"03bb","state":"submitted","message":{"role":"user","parts":[{"type":"text","text":"hello"}]}}"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "abc");
        assert!(task.session_id.is_none());
        assert!(task.payload_cid.is_none());
        assert!(task.payment_tx.is_none());
        assert!(task.payment_amount.is_none());
        assert!(task.result.is_none());
    }

    #[test]
    fn test_task_unique_ids() {
        let t1 = Task::new("02aa", "03bb", "hello");
        let t2 = Task::new("02aa", "03bb", "hello");
        assert_ne!(t1.id, t2.id, "each task should get a unique UUID");
    }

    #[test]
    fn test_presence_with_signature_roundtrip() {
        let ann = PresenceAnnouncement {
            agent_id: "02abcdef".to_string(),
            name: "signed".to_string(),
            capabilities: vec![],
            waku_topic: "/test/proto".to_string(),
            ttl_secs: 60,
            signature: Some(vec![1, 2, 3, 4]),
        };
        let json = serde_json::to_string(&ann).unwrap();
        assert!(json.contains("signature"));
        let deserialized: PresenceAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.signature, Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn test_task_state_deserialization() {
        let state: TaskState = serde_json::from_str("\"input_required\"").unwrap();
        assert_eq!(state, TaskState::InputRequired);
        let state: TaskState = serde_json::from_str("\"cancelled\"").unwrap();
        assert_eq!(state, TaskState::Cancelled);
    }

    #[test]
    fn test_part_text_tagged_serialization() {
        let part = Part::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        let deserialized: Part = serde_json::from_str(&json).unwrap();
        assert_eq!(part, deserialized);
    }

    #[test]
    fn test_message_multi_part() {
        let msg = Message {
            role: "user".to_string(),
            parts: vec![
                Part::Text {
                    text: "first".to_string(),
                },
                Part::Text {
                    text: "second".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.parts.len(), 2);
    }

    #[test]
    fn test_agent_card_empty_capabilities() {
        let card = AgentCard {
            name: "bare".to_string(),
            description: "no caps".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![],
            public_key: "02dead".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert!(deserialized.capabilities.is_empty());
    }

    #[test]
    fn test_respond_clears_payment_fields() {
        let mut task = Task::new("02aa", "03bb", "pay me");
        task.payment_tx = Some("0xabc".to_string());
        task.payment_amount = Some(100);
        let response = task.respond("done");
        // respond() creates a new task — payment fields should be None
        assert!(response.payment_tx.is_none());
        assert!(response.payment_amount.is_none());
    }

    #[test]
    fn test_respond_clears_payload_cid() {
        let mut task = Task::new("02aa", "03bb", "big data");
        task.payload_cid = Some("zQmBig".to_string());
        let response = task.respond("got it");
        assert!(response.payload_cid.is_none());
    }
}
