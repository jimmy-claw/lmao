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
}
