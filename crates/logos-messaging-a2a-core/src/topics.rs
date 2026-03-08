//! Waku content topic helpers.

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
}
