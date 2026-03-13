use logos_messaging_a2a_crypto::EncryptedPayload;
use serde::{Deserialize, Serialize};

use crate::agent::AgentCard;
use crate::presence::PresenceAnnouncement;
use crate::task::{Task, TaskStreamChunk};

/// Wire envelope for all messages on Waku topics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum A2AEnvelope {
    /// Agent discovery advertisement — broadcast on the discovery topic so
    /// peers learn about available agents and their capabilities.
    AgentCard(AgentCard),
    /// A plaintext task sent from one agent to another.
    Task(Task),
    /// Delivery acknowledgement for a previously received message.
    Ack {
        /// Unique identifier of the message being acknowledged.
        message_id: String,
    },
    /// An end-to-end encrypted task payload, opaque to relay nodes.
    EncryptedTask {
        /// The encrypted ciphertext and nonce produced by the sender's
        /// Double Ratchet session.
        encrypted: EncryptedPayload,
        /// Sender's X25519 public key (hex) so the recipient can look up
        /// the correct decryption session.
        sender_pubkey: String,
    },
    /// Ephemeral presence announcement indicating an agent is online.
    Presence(PresenceAnnouncement),
    /// A streaming chunk carrying incremental task output (e.g. LLM tokens).
    StreamChunk(TaskStreamChunk),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;

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
    fn test_stream_chunk_envelope_serialization() {
        let chunk = TaskStreamChunk {
            task_id: "task-42".to_string(),
            chunk_index: 3,
            text: "partial ".to_string(),
            is_final: false,
        };
        let envelope = A2AEnvelope::StreamChunk(chunk.clone());
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("stream_chunk"));
        let deserialized: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, deserialized);
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
    fn test_agent_card_envelope_serialization() {
        let card = crate::agent::AgentCard {
            name: "echo".to_string(),
            description: "Echoes".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02abcdef".to_string(),
            intro_bundle: None,
        };
        let envelope = A2AEnvelope::AgentCard(card.clone());
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("agent_card"));
        let deserialized: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, deserialized);
    }

    #[test]
    fn test_envelope_type_tags_are_snake_case() {
        let task = Task::new("02aa", "03bb", "test");
        let cases: Vec<(A2AEnvelope, &str)> = vec![
            (A2AEnvelope::Task(task), "task"),
            (
                A2AEnvelope::Ack {
                    message_id: "m1".to_string(),
                },
                "ack",
            ),
            (
                A2AEnvelope::EncryptedTask {
                    encrypted: EncryptedPayload {
                        nonce: "n".to_string(),
                        ciphertext: "c".to_string(),
                    },
                    sender_pubkey: "pk".to_string(),
                },
                "encrypted_task",
            ),
            (
                A2AEnvelope::Presence(PresenceAnnouncement {
                    agent_id: "02ab".to_string(),
                    name: "a".to_string(),
                    capabilities: vec![],
                    waku_topic: "/t".to_string(),
                    ttl_secs: 60,
                    signature: None,
                }),
                "presence",
            ),
            (
                A2AEnvelope::StreamChunk(TaskStreamChunk {
                    task_id: "t1".to_string(),
                    chunk_index: 0,
                    text: "x".to_string(),
                    is_final: false,
                }),
                "stream_chunk",
            ),
        ];
        for (envelope, expected_tag) in cases {
            let json = serde_json::to_string(&envelope).unwrap();
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(v["type"].as_str().unwrap(), expected_tag);
        }
    }

    #[test]
    fn test_envelope_invalid_type_tag_fails() {
        let json = r#"{"type":"nonexistent","data":"x"}"#;
        let result = serde_json::from_str::<A2AEnvelope>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_envelope_clone_and_debug() {
        let envelope = A2AEnvelope::Ack {
            message_id: "msg-1".to_string(),
        };
        let cloned = envelope.clone();
        assert_eq!(envelope, cloned);
        let debug = format!("{:?}", envelope);
        assert!(debug.contains("Ack"));
    }

    #[test]
    fn test_envelope_partial_eq_different_variants() {
        let task_env = A2AEnvelope::Task(Task::new("02aa", "03bb", "hi"));
        let ack_env = A2AEnvelope::Ack {
            message_id: "m1".to_string(),
        };
        assert_ne!(task_env, ack_env);
    }

    #[test]
    fn test_ack_envelope_roundtrip_from_json() {
        let json = r#"{"type":"ack","message_id":"uuid-123-456"}"#;
        let envelope: A2AEnvelope = serde_json::from_str(json).unwrap();
        if let A2AEnvelope::Ack { message_id } = &envelope {
            assert_eq!(message_id, "uuid-123-456");
        } else {
            panic!("expected Ack variant");
        }
        let reserialized = serde_json::to_string(&envelope).unwrap();
        let re_deserialized: A2AEnvelope = serde_json::from_str(&reserialized).unwrap();
        assert_eq!(envelope, re_deserialized);
    }

    #[test]
    fn test_encrypted_task_fields_accessible() {
        let envelope = A2AEnvelope::EncryptedTask {
            encrypted: EncryptedPayload {
                nonce: "test_nonce".to_string(),
                ciphertext: "test_cipher".to_string(),
            },
            sender_pubkey: "sender_key".to_string(),
        };
        if let A2AEnvelope::EncryptedTask {
            encrypted,
            sender_pubkey,
        } = &envelope
        {
            assert_eq!(encrypted.nonce, "test_nonce");
            assert_eq!(encrypted.ciphertext, "test_cipher");
            assert_eq!(sender_pubkey, "sender_key");
        } else {
            panic!("expected EncryptedTask variant");
        }
    }
}
