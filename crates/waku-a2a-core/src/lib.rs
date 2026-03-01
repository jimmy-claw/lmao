use serde::{Deserialize, Serialize};
use uuid::Uuid;
use waku_a2a_crypto::{EncryptedPayload, IntroBundle};

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
        }
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
}
