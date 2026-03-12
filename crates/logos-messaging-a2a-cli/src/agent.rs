use anyhow::Result;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::cli::AgentAction;
use crate::common::parse_capabilities;

pub async fn handle(action: AgentAction, transport: LogosMessagingTransport) -> Result<()> {
    match action {
        AgentAction::Run {
            name,
            capabilities,
            encrypt,
            keyfile,
        } => {
            let caps = parse_capabilities(&capabilities);
            let node = if let Some(path) = keyfile {
                println!("Using keyfile: {}", path.display());
                WakuA2ANode::from_keyfile(
                    &name,
                    &format!("{} agent", name),
                    caps,
                    transport,
                    &path,
                )?
            } else if encrypt {
                WakuA2ANode::new_encrypted(&name, &format!("{} agent", name), caps, transport)
            } else {
                WakuA2ANode::new(&name, &format!("{} agent", name), caps, transport)
            };
            println!("Agent: {}", node.card.name);
            println!("Pubkey: {}", node.pubkey());
            if encrypt {
                let bundle = node.card.intro_bundle.as_ref().unwrap();
                println!("Encryption: ENABLED (X25519+ChaCha20-Poly1305)");
                println!("X25519 pubkey: {}", bundle.agent_pubkey);
            }
            println!("Listening for tasks...\n");

            // Announce on startup
            if let Err(e) = node.announce().await {
                eprintln!("Warning: announce failed (is nwaku running?): {}", e);
            }

            // Poll loop
            loop {
                match node.poll_tasks().await {
                    Ok(tasks) => {
                        for task in tasks {
                            println!("Received task {} from {}", task.id, task.from);
                            if let Some(text) = task.text() {
                                println!("  Message: {}", text);
                                // Echo behavior by default
                                let response = format!("Echo: {}", text);
                                if let Err(e) = node.respond(&task, &response).await {
                                    eprintln!("  Failed to respond: {}", e);
                                } else {
                                    println!("  Responded: {}", response);
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
        }
        AgentAction::Discover => {
            let node = WakuA2ANode::new("discovery-client", "temporary", vec![], transport);
            match node.discover().await {
                Ok(cards) => {
                    if cards.is_empty() {
                        println!("No agents found. (Are agents announcing on the network?)");
                    } else {
                        println!("Discovered {} agent(s):\n", cards.len());
                        for card in cards {
                            println!("  Name: {}", card.name);
                            println!("  Description: {}", card.description);
                            println!("  Capabilities: {}", card.capabilities.join(", "));
                            println!("  Pubkey: {}", card.public_key);
                            if let Some(ref bundle) = card.intro_bundle {
                                println!("  Encryption: YES (X25519: {})", bundle.agent_pubkey);
                            }
                            println!();
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Discovery failed (is nwaku running?): {}", e);
                }
            }
        }
        AgentAction::Bundle => {
            let node = WakuA2ANode::new_encrypted("bundle-gen", "temporary", vec![], transport);
            let bundle = node.card.intro_bundle.as_ref().unwrap();
            let json = serde_json::to_string_pretty(bundle)?;
            println!("{}", json);
        }
    }
    Ok(())
}
