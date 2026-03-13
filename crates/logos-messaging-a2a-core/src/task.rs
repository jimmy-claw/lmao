use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Task lifecycle states (A2A spec).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// The task has been created and sent but the recipient has not yet
    /// started processing it.
    Submitted,
    /// The recipient agent is actively working on the task.
    Working,
    /// The agent needs additional input from the requester before it can
    /// continue (e.g. clarification or confirmation).
    InputRequired,
    /// The agent has finished processing and produced a result.
    Completed,
    /// The task failed due to an error on the agent side.
    Failed,
    /// The task was explicitly cancelled by the requester or the agent.
    Cancelled,
}

/// A message part. Text-only for v0.1; extensible to images, files, etc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    /// A plain-text content part.
    Text {
        /// The UTF-8 text content of this part.
        text: String,
    },
}

/// A message within a task (user or agent turn).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// The role of the message author — typically `"user"` for the requester
    /// or `"agent"` for the responding agent.
    pub role: String,
    /// Ordered list of content parts that make up this message.
    pub parts: Vec<Part>,
}

/// A streaming chunk for incremental task results (e.g., LLM token output).
///
/// Agents send a sequence of chunks with incrementing `chunk_index`.
/// The final chunk has `is_final = true`, signalling the stream is complete.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskStreamChunk {
    /// Identifier of the task this chunk belongs to.
    pub task_id: String,
    /// Zero-based sequence number for ordering chunks within a stream.
    pub chunk_index: u32,
    /// The incremental text payload of this chunk (e.g. one or more LLM tokens).
    pub text: String,
    /// When `true`, this is the last chunk in the stream and the task
    /// result is now complete.
    pub is_final: bool,
}

/// An A2A task: the unit of work exchanged between agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    /// Globally unique task identifier (UUID v4).
    pub id: String,
    /// Public key (compressed secp256k1 hex) of the agent that created this task.
    pub from: String,
    /// Public key (compressed secp256k1 hex) of the intended recipient agent.
    pub to: String,
    /// Current lifecycle state of this task.
    pub state: TaskState,
    /// The original request message submitted by the sender.
    pub message: Message,
    /// The agent's response message, populated when the task reaches a
    /// terminal state such as [`TaskState::Completed`].
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
    /// Create a new task in the [`TaskState::Submitted`] state with a single
    /// text part. A random UUID is assigned as the task id.
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

    /// Build a completed response task that mirrors the original task's id
    /// and session, swaps `from`/`to`, and attaches the given text as the
    /// agent's result message.
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

    /// Extract the text content of the first part in the request message,
    /// or `None` if the message has no parts.
    pub fn text(&self) -> Option<&str> {
        self.message
            .parts
            .iter()
            .map(|p| match p {
                Part::Text { text } => text.as_str(),
            })
            .next()
    }

    /// Extract the text content of the first part in the result message,
    /// or `None` if no result has been set yet.
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

    #[test]
    fn test_stream_chunk_serialization() {
        let chunk = TaskStreamChunk {
            task_id: "task-1".to_string(),
            chunk_index: 0,
            text: "Hello ".to_string(),
            is_final: false,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("\"task_id\":\"task-1\""));
        assert!(json.contains("\"chunk_index\":0"));
        assert!(json.contains("\"is_final\":false"));
        let deserialized: TaskStreamChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(chunk, deserialized);
    }

    #[test]
    fn test_stream_chunk_final() {
        let chunk = TaskStreamChunk {
            task_id: "task-1".to_string(),
            chunk_index: 5,
            text: "done".to_string(),
            is_final: true,
        };
        assert!(chunk.is_final);
        assert_eq!(chunk.chunk_index, 5);
    }

    #[test]
    fn test_task_empty_text() {
        let task = Task::new("02aa", "03bb", "");
        assert_eq!(task.text(), Some(""));
    }

    #[test]
    fn test_task_long_text() {
        let long_text = "x".repeat(10_000);
        let task = Task::new("02aa", "03bb", &long_text);
        assert_eq!(task.text(), Some(long_text.as_str()));
        // Roundtrip through JSON
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.text(), Some(long_text.as_str()));
    }

    #[test]
    fn test_task_full_roundtrip() {
        let mut task = Task::new("02aa", "03bb", "hello");
        task.session_id = Some("sess-1".to_string());
        task.payload_cid = Some("cid-1".to_string());
        task.payment_tx = Some("tx-1".to_string());
        task.payment_amount = Some(100);
        task.result = Some(Message {
            role: "agent".to_string(),
            parts: vec![Part::Text {
                text: "world".to_string(),
            }],
        });
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(task, deserialized);
    }

    #[test]
    fn test_task_state_all_variants_roundtrip() {
        let all_states = vec![
            TaskState::Submitted,
            TaskState::Working,
            TaskState::InputRequired,
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Cancelled,
        ];
        for state in all_states {
            let json = serde_json::to_string(&state).unwrap();
            let deserialized: TaskState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, deserialized);
        }
    }

    #[test]
    fn test_task_state_invalid_value() {
        let result = serde_json::from_str::<TaskState>("\"invalid_state\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_message_empty_parts() {
        let msg = Message {
            role: "user".to_string(),
            parts: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert!(deserialized.parts.is_empty());
    }

    #[test]
    fn test_text_on_empty_parts() {
        let task = Task {
            id: "test".to_string(),
            from: "a".to_string(),
            to: "b".to_string(),
            state: TaskState::Submitted,
            message: Message {
                role: "user".to_string(),
                parts: vec![],
            },
            result: None,
            session_id: None,
            payload_cid: None,
            payment_tx: None,
            payment_amount: None,
        };
        assert!(task.text().is_none());
    }

    #[test]
    fn test_result_text_with_populated_result() {
        let mut task = Task::new("a", "b", "hello");
        task.result = Some(Message {
            role: "agent".to_string(),
            parts: vec![Part::Text {
                text: "response".to_string(),
            }],
        });
        assert_eq!(task.result_text(), Some("response"));
    }

    #[test]
    fn test_result_text_empty_parts() {
        let mut task = Task::new("a", "b", "hello");
        task.result = Some(Message {
            role: "agent".to_string(),
            parts: vec![],
        });
        assert!(task.result_text().is_none());
    }

    #[test]
    fn test_stream_chunk_empty_text() {
        let chunk = TaskStreamChunk {
            task_id: "t1".to_string(),
            chunk_index: 0,
            text: "".to_string(),
            is_final: false,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        let deserialized: TaskStreamChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.text, "");
    }

    #[test]
    fn test_stream_chunk_max_index() {
        let chunk = TaskStreamChunk {
            task_id: "t1".to_string(),
            chunk_index: u32::MAX,
            text: "last".to_string(),
            is_final: true,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        let deserialized: TaskStreamChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.chunk_index, u32::MAX);
    }

    #[test]
    fn test_task_clone_and_debug() {
        let task = Task::new("02aa", "03bb", "test");
        let cloned = task.clone();
        assert_eq!(task, cloned);
        let debug = format!("{:?}", task);
        assert!(debug.contains("Task"));
        assert!(debug.contains("02aa"));
    }

    #[test]
    fn test_task_state_debug() {
        assert!(format!("{:?}", TaskState::Submitted).contains("Submitted"));
        assert!(format!("{:?}", TaskState::Working).contains("Working"));
        assert!(format!("{:?}", TaskState::InputRequired).contains("InputRequired"));
        assert!(format!("{:?}", TaskState::Completed).contains("Completed"));
        assert!(format!("{:?}", TaskState::Failed).contains("Failed"));
        assert!(format!("{:?}", TaskState::Cancelled).contains("Cancelled"));
    }

    #[test]
    fn test_respond_preserves_original_message() {
        let task = Task::new("02aa", "03bb", "original question");
        let response = task.respond("the answer");
        assert_eq!(response.text(), Some("original question"));
        assert_eq!(response.result_text(), Some("the answer"));
    }

    #[test]
    fn test_payment_amount_max() {
        let mut task = Task::new("a", "b", "expensive");
        task.payment_amount = Some(u64::MAX);
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.payment_amount, Some(u64::MAX));
    }

    #[test]
    fn test_task_partial_eq() {
        let t1 = Task::new("a", "b", "hello");
        let t2 = Task::new("a", "b", "hello");
        // Different UUIDs, so not equal
        assert_ne!(t1, t2);
        // Same task cloned should be equal
        let t3 = t1.clone();
        assert_eq!(t1, t3);
    }

    #[test]
    fn test_stream_chunk_clone() {
        let chunk = TaskStreamChunk {
            task_id: "t1".to_string(),
            chunk_index: 0,
            text: "hello".to_string(),
            is_final: false,
        };
        let cloned = chunk.clone();
        assert_eq!(chunk, cloned);
    }

    #[test]
    fn test_part_clone_and_debug() {
        let part = Part::Text {
            text: "hello".to_string(),
        };
        let cloned = part.clone();
        assert_eq!(part, cloned);
        let debug = format!("{:?}", part);
        assert!(debug.contains("Text"));
    }

    #[test]
    fn test_message_clone_and_debug() {
        let msg = Message {
            role: "user".to_string(),
            parts: vec![Part::Text {
                text: "hi".to_string(),
            }],
        };
        let cloned = msg.clone();
        assert_eq!(msg, cloned);
        let debug = format!("{:?}", msg);
        assert!(debug.contains("Message"));
    }
}
