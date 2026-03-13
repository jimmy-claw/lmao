//! Waku content topic helpers.

/// Well-known Waku content topic for agent discovery broadcasts.
/// All agents publish their [`AgentCard`](crate::AgentCard) here on startup.
pub const DISCOVERY: &str = "/waku-a2a/1/discovery/proto";

/// Well-known topic for presence announcements.
/// All agents subscribe on startup to discover live peers.
pub const PRESENCE: &str = "/lmao/1/presence/proto";

/// Returns the Waku content topic where a specific agent receives tasks.
///
/// Each agent listens on a topic derived from its compressed secp256k1
/// public key, so senders address tasks by the recipient's identity.
pub fn task_topic(recipient_pubkey: &str) -> String {
    format!("/waku-a2a/1/task/{}/proto", recipient_pubkey)
}

/// Returns the Waku content topic for delivery acknowledgements of a
/// specific message. The sender subscribes here to confirm the recipient
/// received the message identified by `message_id`.
pub fn ack_topic(message_id: &str) -> String {
    format!("/waku-a2a/1/ack/{}/proto", message_id)
}

/// Returns the Waku content topic for streaming chunks of a specific task.
/// The requesting agent subscribes here to receive incremental output
/// (e.g. LLM tokens) for the task identified by `task_id`.
pub fn stream_topic(task_id: &str) -> String {
    format!("/waku-a2a/1/stream/{}/proto", task_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topics() {
        assert_eq!(DISCOVERY, "/waku-a2a/1/discovery/proto");
        assert_eq!(task_topic("02abcdef"), "/waku-a2a/1/task/02abcdef/proto");
        assert_eq!(ack_topic("msg-123"), "/waku-a2a/1/ack/msg-123/proto");
    }

    #[test]
    fn test_presence_topic() {
        assert_eq!(PRESENCE, "/lmao/1/presence/proto");
    }

    #[test]
    fn test_stream_topic() {
        assert_eq!(stream_topic("abc-123"), "/waku-a2a/1/stream/abc-123/proto");
    }

    #[test]
    fn test_task_topic_empty_key() {
        assert_eq!(task_topic(""), "/waku-a2a/1/task//proto");
    }

    #[test]
    fn test_ack_topic_empty_id() {
        assert_eq!(ack_topic(""), "/waku-a2a/1/ack//proto");
    }

    #[test]
    fn test_stream_topic_empty_id() {
        assert_eq!(stream_topic(""), "/waku-a2a/1/stream//proto");
    }

    #[test]
    fn test_task_topic_special_characters() {
        assert_eq!(
            task_topic("key/with/slashes"),
            "/waku-a2a/1/task/key/with/slashes/proto"
        );
    }

    #[test]
    fn test_topics_are_distinct() {
        let id = "02abcdef";
        let task = task_topic(id);
        let ack = ack_topic(id);
        let stream = stream_topic(id);
        assert_ne!(task, ack);
        assert_ne!(task, stream);
        assert_ne!(ack, stream);
        assert_ne!(task, DISCOVERY);
        assert_ne!(task, PRESENCE);
    }

    #[test]
    fn test_topic_constants_start_with_slash() {
        assert!(DISCOVERY.starts_with('/'));
        assert!(PRESENCE.starts_with('/'));
    }

    #[test]
    fn test_topic_functions_return_slash_prefixed() {
        assert!(task_topic("x").starts_with('/'));
        assert!(ack_topic("x").starts_with('/'));
        assert!(stream_topic("x").starts_with('/'));
    }

    #[test]
    fn test_topic_functions_end_with_proto() {
        assert!(task_topic("x").ends_with("/proto"));
        assert!(ack_topic("x").ends_with("/proto"));
        assert!(stream_topic("x").ends_with("/proto"));
        assert!(DISCOVERY.ends_with("/proto"));
        assert!(PRESENCE.ends_with("/proto"));
    }

    #[test]
    fn test_long_pubkey_topic() {
        let long_key = "a".repeat(200);
        let topic = task_topic(&long_key);
        assert!(topic.contains(&long_key));
    }
}
