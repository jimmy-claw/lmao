use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
