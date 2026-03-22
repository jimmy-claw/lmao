use anyhow::Result;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use std::path::PathBuf;

pub fn parse_capabilities(capabilities: &str) -> Vec<String> {
    capabilities
        .split(',')
        .map(|s| s.trim().to_string())
        .collect()
}

/// Identity configuration extracted from global CLI flags.
#[derive(Debug, Clone)]
pub struct IdentityConfig {
    pub keyfile: Option<PathBuf>,
    pub encrypt: bool,
}

/// Build a [`WakuA2ANode`] using the global identity flags.
///
/// When `--keyfile` is provided, the node loads (or creates) a persistent
/// signing key so that every CLI invocation shares the same pubkey and
/// can therefore poll for responses to tasks it previously sent.
///
/// When `--encrypt` is set, the node generates an X25519 keypair for
/// end-to-end encryption.
pub fn build_node(
    name: &str,
    description: &str,
    capabilities: Vec<String>,
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
) -> Result<WakuA2ANode<LogosMessagingTransport>> {
    let node = if let Some(ref path) = identity.keyfile {
        WakuA2ANode::from_keyfile(name, description, capabilities, transport, path)?
    } else if identity.encrypt {
        WakuA2ANode::new_encrypted(name, description, capabilities, transport)
    } else {
        WakuA2ANode::new(name, description, capabilities, transport)
    };
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_capability() {
        assert_eq!(parse_capabilities("text"), vec!["text"]);
    }

    #[test]
    fn parse_multiple_capabilities() {
        assert_eq!(
            parse_capabilities("text, code, search"),
            vec!["text", "code", "search"]
        );
    }

    #[test]
    fn build_node_ephemeral() {
        let transport = LogosMessagingTransport::new("http://localhost:8645");
        let id = IdentityConfig {
            keyfile: None,
            encrypt: false,
        };
        let node = build_node("test", "test node", vec![], transport, &id).unwrap();
        assert!(!node.pubkey().is_empty());
    }

    #[test]
    fn build_node_encrypted() {
        let transport = LogosMessagingTransport::new("http://localhost:8645");
        let id = IdentityConfig {
            keyfile: None,
            encrypt: true,
        };
        let node = build_node("test", "test node", vec![], transport, &id).unwrap();
        assert!(node.card.intro_bundle.is_some());
    }

    #[test]
    fn build_node_with_keyfile() {
        let dir = tempfile::tempdir().unwrap();
        let keypath = dir.path().join("test.key");
        let transport = LogosMessagingTransport::new("http://localhost:8645");
        let id = IdentityConfig {
            keyfile: Some(keypath.clone()),
            encrypt: false,
        };
        let node1 = build_node("test", "test node", vec![], transport, &id).unwrap();
        let pk1 = node1.pubkey().to_string();

        // Second build with same keyfile should produce same pubkey
        let transport2 = LogosMessagingTransport::new("http://localhost:8645");
        let node2 = build_node("test", "test node", vec![], transport2, &id).unwrap();
        assert_eq!(pk1, node2.pubkey());
    }
}
