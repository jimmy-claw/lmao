use anyhow::Result;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use std::collections::HashSet;

use crate::cli::PresenceAction;
use crate::common::{build_node, parse_capabilities, IdentityConfig};

fn print_peer(agent_id: &str, info: &logos_messaging_a2a_node::presence::PeerInfo) {
    let expired = if info.is_expired() { " [EXPIRED]" } else { "" };
    println!("  Name:         {}", info.name);
    println!("  Capabilities: {}", info.capabilities.join(", "));
    println!("  Pubkey:       {}", agent_id);
    println!("  Waku topic:   {}", info.waku_topic);
    println!("  Last seen:    {}", info.last_seen);
    println!("  Status:       TTL {}s{}", info.ttl_secs, expired);
    println!();
}

fn peer_to_json(
    agent_id: &str,
    info: &logos_messaging_a2a_node::presence::PeerInfo,
) -> serde_json::Value {
    serde_json::json!({
        "name": info.name,
        "capabilities": info.capabilities,
        "pubkey": agent_id,
        "waku_topic": info.waku_topic,
        "last_seen": info.last_seen.to_string(),
        "ttl_secs": info.ttl_secs,
        "expired": info.is_expired(),
    })
}

pub async fn handle(
    action: PresenceAction,
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
    json: bool,
) -> Result<()> {
    match action {
        PresenceAction::Announce {
            name,
            capabilities,
            ttl,
            repeat,
        } => {
            let caps = parse_capabilities(&capabilities);
            let node = build_node(&name, &format!("{} agent", name), caps, transport, identity)?;

            if !json {
                println!("Announcing presence: {}", node.card.name);
                println!("Pubkey: {}", node.pubkey());
                println!("TTL: {}s", ttl);
                if identity.encrypt {
                    println!("Encryption: ENABLED");
                }
            }

            match node.announce_presence_with_ttl(ttl).await {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "event": "announced",
                                "name": node.card.name,
                                "pubkey": node.pubkey(),
                                "ttl_secs": ttl,
                                "encryption": identity.encrypt,
                            }))?
                        );
                    } else {
                        println!("Presence announced.");
                    }
                }
                Err(e) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "event": "announce_failed",
                                "error": e.to_string(),
                            }))?
                        );
                    } else {
                        eprintln!("Announce failed (is nwaku running?): {}", e);
                    }
                    return Ok(());
                }
            }

            if repeat {
                let interval = std::cmp::max(ttl / 2, 1);
                if !json {
                    println!("Re-announcing every {}s (Ctrl-C to stop)\n", interval);
                }
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                    match node.announce_presence_with_ttl(ttl).await {
                        Ok(()) => {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string(&serde_json::json!({
                                        "event": "re_announced",
                                        "name": node.card.name,
                                        "pubkey": node.pubkey(),
                                        "ttl_secs": ttl,
                                    }))?
                                );
                            } else {
                                println!("Re-announced presence.");
                            }
                        }
                        Err(e) => eprintln!("Re-announce failed: {}", e),
                    }
                }
            }
        }
        PresenceAction::Discover {
            capability,
            watch,
            timeout,
        } => {
            let node = build_node(
                "presence-discover",
                "temporary",
                vec![],
                transport,
                identity,
            )?;

            if watch {
                if !json {
                    println!("Watching for presence announcements (Ctrl-C to stop)...\n");
                }
                let mut seen = HashSet::new();
                loop {
                    match node.poll_presence().await {
                        Ok(count) => {
                            if count > 0 {
                                let peers = match &capability {
                                    Some(cap) => node.find_peers_by_capability(cap),
                                    None => node.peers().all_live(),
                                };
                                for (id, info) in &peers {
                                    if seen.insert(format!("{}-{}", id, info.last_seen)) {
                                        if json {
                                            println!(
                                                "{}",
                                                serde_json::to_string(&peer_to_json(id, info))?
                                            );
                                        } else {
                                            print_peer(id, info);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Poll error (is nwaku running?): {}", e);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            } else {
                if !json {
                    println!("Listening for presence announcements ({}s)...\n", timeout);
                }
                let deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
                while tokio::time::Instant::now() < deadline {
                    if let Err(e) = node.poll_presence().await {
                        eprintln!("Poll error (is nwaku running?): {}", e);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }

                let peers = match &capability {
                    Some(cap) => node.find_peers_by_capability(cap),
                    None => node.peers().all_live(),
                };

                if json {
                    let items: Vec<_> = peers
                        .iter()
                        .map(|(id, info)| peer_to_json(id, info))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({ "peers": items }))?
                    );
                } else if peers.is_empty() {
                    println!("No peers found.");
                } else {
                    println!("Found {} peer(s):\n", peers.len());
                    for (id, info) in &peers {
                        print_peer(id, info);
                    }
                }
            }
        }
        PresenceAction::Peers {
            capability,
            watch,
            timeout,
        } => {
            let node = build_node("presence-peers", "temporary", vec![], transport, identity)?;

            if watch {
                if !json {
                    println!("Watching for unique peers (Ctrl-C to stop)...\n");
                }
                let mut known_ids = HashSet::new();
                loop {
                    match node.poll_presence().await {
                        Ok(count) => {
                            if count > 0 {
                                let peers = match &capability {
                                    Some(cap) => node.find_peers_by_capability(cap),
                                    None => node.peers().all_live(),
                                };
                                for (id, info) in &peers {
                                    if known_ids.insert(id.clone()) {
                                        if json {
                                            println!(
                                                "{}",
                                                serde_json::to_string(&peer_to_json(id, info))?
                                            );
                                        } else {
                                            print_peer(id, info);
                                        }
                                    }
                                }
                                if !json {
                                    println!("--- {} unique peer(s) ---\n", known_ids.len());
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Poll error (is nwaku running?): {}", e);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            } else {
                if !json {
                    println!("Discovering unique peers ({}s)...\n", timeout);
                }
                let deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
                while tokio::time::Instant::now() < deadline {
                    if let Err(e) = node.poll_presence().await {
                        eprintln!("Poll error (is nwaku running?): {}", e);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }

                let peers = match &capability {
                    Some(cap) => node.find_peers_by_capability(cap),
                    None => node.peers().all_live(),
                };

                if json {
                    let items: Vec<_> = peers
                        .iter()
                        .map(|(id, info)| peer_to_json(id, info))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({ "peers": items }))?
                    );
                } else if peers.is_empty() {
                    println!("No peers found.");
                } else {
                    println!("Found {} unique peer(s):\n", peers.len());
                    for (id, info) in &peers {
                        print_peer(id, info);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_messaging_a2a_node::presence::PeerInfo;

    #[test]
    fn peer_to_json_is_parseable() {
        let info = PeerInfo {
            name: "echo-agent".to_string(),
            capabilities: vec!["text".to_string(), "code".to_string()],
            waku_topic: "/waku/2/a2a-echo/proto".to_string(),
            ttl_secs: 300,
            last_seen: 1700000000,
        };
        let value = peer_to_json("02abcdef", &info);
        let output = serde_json::to_string(&value).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["name"], "echo-agent");
        assert_eq!(parsed["pubkey"], "02abcdef");
        assert_eq!(parsed["ttl_secs"], 300);
        let caps = parsed["capabilities"].as_array().unwrap();
        assert_eq!(caps.len(), 2);
    }

    #[test]
    fn presence_discover_json_output_is_parseable() {
        let items = vec![serde_json::json!({
            "name": "bot",
            "capabilities": ["text"],
            "pubkey": "02aabb",
            "waku_topic": "/waku/2/topic/proto",
            "last_seen": "1700000000",
            "ttl_secs": 300,
            "expired": false,
        })];
        let output = serde_json::to_string(&serde_json::json!({ "peers": items })).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let peers = parsed["peers"].as_array().unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0]["name"], "bot");
    }
}
