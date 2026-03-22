use anyhow::Result;
use logos_messaging_a2a_core::topics;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::common::{build_node, IdentityConfig};

/// Display agent identity and topic configuration.
pub fn handle(
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
    json: bool,
) -> Result<()> {
    let node = build_node("info", "info command", vec![], transport, identity)?;
    let pubkey = node.pubkey().to_string();
    let task_topic = topics::task_topic(&pubkey);
    let discovery_topic = topics::DISCOVERY;
    let presence_topic = topics::PRESENCE;
    let encrypt = identity.encrypt || node.card.intro_bundle.is_some();

    if json {
        let obj = serde_json::json!({
            "public_key": pubkey,
            "task_topic": task_topic,
            "discovery_topic": discovery_topic,
            "presence_topic": presence_topic,
            "encryption": encrypt,
        });
        println!("{}", serde_json::to_string(&obj)?);
    } else {
        if let Some(ref kf) = identity.keyfile {
            eprintln!("Keyfile: {}", kf.display());
        } else {
            eprintln!("Identity: ephemeral (use --keyfile for persistent identity)");
        }
        println!("Public key:      {pubkey}");
        println!("Task topic:      {task_topic}");
        println!("Discovery topic: {discovery_topic}");
        println!("Presence topic:  {presence_topic}");
        println!(
            "Encryption:      {}",
            if encrypt { "enabled" } else { "disabled" }
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a node and return the info fields as JSON.
    fn info_json(encrypt: bool) -> serde_json::Value {
        let transport = LogosMessagingTransport::new("http://localhost:8645");
        let identity = IdentityConfig {
            keyfile: None,
            encrypt,
        };
        let node = build_node("info", "info command", vec![], transport, &identity).unwrap();
        let pubkey = node.pubkey().to_string();
        serde_json::json!({
            "public_key": pubkey,
            "task_topic": topics::task_topic(&pubkey),
            "discovery_topic": topics::DISCOVERY,
            "presence_topic": topics::PRESENCE,
            "encryption": encrypt || node.card.intro_bundle.is_some(),
        })
    }

    #[test]
    fn json_output_contains_all_fields() {
        let obj = info_json(false);
        assert!(obj["public_key"].is_string());
        assert!(obj["task_topic"]
            .as_str()
            .unwrap()
            .contains("/waku-a2a/1/task/"));
        assert_eq!(obj["discovery_topic"], topics::DISCOVERY);
        assert_eq!(obj["presence_topic"], topics::PRESENCE);
        assert_eq!(obj["encryption"], false);
    }

    #[test]
    fn json_output_encryption_enabled() {
        let obj = info_json(true);
        assert_eq!(obj["encryption"], true);
    }

    #[test]
    fn json_output_is_parseable() {
        let obj = info_json(false);
        let serialized = serde_json::to_string(&obj).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed["public_key"], obj["public_key"]);
    }

    #[test]
    fn task_topic_includes_pubkey() {
        let obj = info_json(false);
        let pubkey = obj["public_key"].as_str().unwrap();
        let task_topic = obj["task_topic"].as_str().unwrap();
        assert!(task_topic.contains(pubkey));
    }

    #[test]
    fn keyfile_identity_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let keypath = dir.path().join("test.key");
        let transport1 = LogosMessagingTransport::new("http://localhost:8645");
        let identity = IdentityConfig {
            keyfile: Some(keypath.clone()),
            encrypt: false,
        };
        let node1 = build_node("info", "test", vec![], transport1, &identity).unwrap();
        let pk1 = node1.pubkey().to_string();

        let transport2 = LogosMessagingTransport::new("http://localhost:8645");
        let node2 = build_node("info", "test", vec![], transport2, &identity).unwrap();
        let pk2 = node2.pubkey().to_string();

        assert_eq!(pk1, pk2);
    }
}
