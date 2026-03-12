use anyhow::Result;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use std::collections::HashSet;

use crate::cli::PresenceAction;
use crate::common::parse_capabilities;

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

pub async fn handle(action: PresenceAction, transport: LogosMessagingTransport) -> Result<()> {
    match action {
        PresenceAction::Announce {
            name,
            capabilities,
            ttl,
            repeat,
            encrypt,
        } => {
            let caps = parse_capabilities(&capabilities);
            let node = if encrypt {
                WakuA2ANode::new_encrypted(&name, &format!("{} agent", name), caps, transport)
            } else {
                WakuA2ANode::new(&name, &format!("{} agent", name), caps, transport)
            };

            println!("Announcing presence: {}", node.card.name);
            println!("Pubkey: {}", node.pubkey());
            println!("TTL: {}s", ttl);
            if encrypt {
                println!("Encryption: ENABLED");
            }

            match node.announce_presence_with_ttl(ttl).await {
                Ok(()) => println!("Presence announced."),
                Err(e) => {
                    eprintln!("Announce failed (is nwaku running?): {}", e);
                    return Ok(());
                }
            }

            if repeat {
                let interval = std::cmp::max(ttl / 2, 1);
                println!("Re-announcing every {}s (Ctrl-C to stop)\n", interval);
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                    match node.announce_presence_with_ttl(ttl).await {
                        Ok(()) => println!("Re-announced presence."),
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
            let node = WakuA2ANode::new("presence-discover", "temporary", vec![], transport);

            if watch {
                println!("Watching for presence announcements (Ctrl-C to stop)...\n");
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
                                        print_peer(id, info);
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
                println!("Listening for presence announcements ({}s)...\n", timeout);
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

                if peers.is_empty() {
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
            let node = WakuA2ANode::new("presence-peers", "temporary", vec![], transport);

            if watch {
                println!("Watching for unique peers (Ctrl-C to stop)...\n");
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
                                        print_peer(id, info);
                                    }
                                }
                                println!("--- {} unique peer(s) ---\n", known_ids.len());
                            }
                        }
                        Err(e) => {
                            eprintln!("Poll error (is nwaku running?): {}", e);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            } else {
                println!("Discovering unique peers ({}s)...\n", timeout);
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

                if peers.is_empty() {
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
