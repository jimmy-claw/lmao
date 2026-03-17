use anyhow::Result;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::common::{build_node, IdentityConfig};

/// Display the node's operational metrics snapshot.
pub fn handle(
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
    json: bool,
) -> Result<()> {
    let node = build_node("metrics", "metrics command", vec![], transport, identity)?;
    let snap = node.metrics().snapshot();

    if json {
        println!("{}", serde_json::to_string(&snap)?);
    } else {
        println!("tasks_sent:              {}", snap.tasks_sent);
        println!("tasks_received:          {}", snap.tasks_received);
        println!("tasks_failed:            {}", snap.tasks_failed);
        println!("messages_published:      {}", snap.messages_published);
        println!("discovery_events:        {}", snap.discovery_events);
        println!("presence_announcements:  {}", snap.presence_announcements);
        println!("encryption_ops:          {}", snap.encryption_ops);
        println!("sessions_created:        {}", snap.sessions_created);
        println!("retry_attempts:          {}", snap.retry_attempts);
        println!("retry_exhaustions:       {}", snap.retry_exhaustions);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_messaging_a2a_node::MetricsSnapshot;

    /// Build a fresh node and return its metrics snapshot.
    fn fresh_snapshot() -> MetricsSnapshot {
        let transport = LogosMessagingTransport::new("http://localhost:8645");
        let identity = IdentityConfig {
            keyfile: None,
            encrypt: false,
        };
        let node = build_node("metrics", "test", vec![], transport, &identity).unwrap();
        node.metrics().snapshot()
    }

    #[test]
    fn fresh_node_metrics_are_zeroed() {
        let snap = fresh_snapshot();
        assert_eq!(snap.tasks_sent, 0);
        assert_eq!(snap.tasks_received, 0);
        assert_eq!(snap.tasks_failed, 0);
        assert_eq!(snap.messages_published, 0);
        assert_eq!(snap.discovery_events, 0);
        assert_eq!(snap.presence_announcements, 0);
        assert_eq!(snap.encryption_ops, 0);
        assert_eq!(snap.sessions_created, 0);
        assert_eq!(snap.retry_attempts, 0);
        assert_eq!(snap.retry_exhaustions, 0);
    }

    #[test]
    fn snapshot_serializes_to_valid_json() {
        let snap = fresh_snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_object());
        assert!(parsed.get("tasks_sent").is_some());
        assert!(parsed.get("retry_exhaustions").is_some());
    }

    #[test]
    fn snapshot_json_contains_all_fields() {
        let snap = fresh_snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let expected_fields = [
            "tasks_sent",
            "tasks_received",
            "tasks_failed",
            "messages_published",
            "discovery_events",
            "presence_announcements",
            "encryption_ops",
            "sessions_created",
            "retry_attempts",
            "retry_exhaustions",
        ];
        for field in &expected_fields {
            assert!(parsed.get(field).is_some(), "missing field: {}", field);
        }
    }
}
