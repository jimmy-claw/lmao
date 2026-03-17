use anyhow::Result;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::common::IdentityConfig;

pub async fn handle(
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
    json: bool,
) -> Result<()> {
    let node = crate::common::build_node("metrics", "metrics probe", vec![], transport, identity)?;
    print_metrics(&node, json)
}

fn print_metrics(node: &WakuA2ANode<LogosMessagingTransport>, json: bool) -> Result<()> {
    let snap = node.metrics();

    if json {
        println!("{}", serde_json::to_string(&snap)?);
    } else {
        println!("Tasks sent:             {}", snap.tasks_sent);
        println!("Tasks received:         {}", snap.tasks_received);
        println!("Tasks failed:           {}", snap.tasks_failed);
        println!("Messages published:     {}", snap.messages_published);
        println!("Messages received:      {}", snap.messages_received);
        println!("Discoveries:            {}", snap.discoveries);
        println!("Announcements sent:     {}", snap.announcements_sent);
        println!("Peers discovered:       {}", snap.peers_discovered);
        println!("Encryptions:            {}", snap.encryptions);
        println!("Decryptions:            {}", snap.decryptions);
        println!("Sessions created:       {}", snap.sessions_created);
        println!("Delegations sent:       {}", snap.delegations_sent);
        println!("Stream chunks sent:     {}", snap.stream_chunks_sent);
        println!("Stream chunks received: {}", snap.stream_chunks_received);
        println!("Retry attempts:         {}", snap.retry_attempts);
        println!("Retries exhausted:      {}", snap.retries_exhausted);
        println!("Responses sent:         {}", snap.responses_sent);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use logos_messaging_a2a_node::MetricsSnapshot;

    #[test]
    fn metrics_json_output_is_parseable() {
        let snap = MetricsSnapshot {
            tasks_sent: 10,
            tasks_received: 5,
            tasks_failed: 1,
            messages_published: 20,
            messages_received: 15,
            discoveries: 3,
            announcements_sent: 2,
            peers_discovered: 4,
            encryptions: 6,
            decryptions: 7,
            sessions_created: 8,
            delegations_sent: 9,
            stream_chunks_sent: 11,
            stream_chunks_received: 12,
            retry_attempts: 13,
            retries_exhausted: 14,
            responses_sent: 16,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tasks_sent"], 10);
        assert_eq!(parsed["tasks_failed"], 1);
        assert_eq!(parsed["responses_sent"], 16);
    }

    #[test]
    fn metrics_snapshot_has_all_fields() {
        let snap = MetricsSnapshot {
            tasks_sent: 0,
            tasks_received: 0,
            tasks_failed: 0,
            messages_published: 0,
            messages_received: 0,
            discoveries: 0,
            announcements_sent: 0,
            peers_discovered: 0,
            encryptions: 0,
            decryptions: 0,
            sessions_created: 0,
            delegations_sent: 0,
            stream_chunks_sent: 0,
            stream_chunks_received: 0,
            retry_attempts: 0,
            retries_exhausted: 0,
            responses_sent: 0,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Verify all 17 fields are present
        assert!(parsed.as_object().unwrap().len() == 17);
    }
}
