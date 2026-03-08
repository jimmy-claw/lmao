use logos_messaging_a2a_crypto::EncryptedPayload;
use serde::{Deserialize, Serialize};

use crate::agent::AgentCard;
use crate::presence::PresenceAnnouncement;
use crate::task::Task;

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
}
