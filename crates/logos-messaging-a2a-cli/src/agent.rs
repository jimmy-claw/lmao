use anyhow::Result;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::cli::AgentAction;
use crate::common::{build_node, parse_capabilities, IdentityConfig};

pub async fn handle(
    action: AgentAction,
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
) -> Result<()> {
    match action {
        AgentAction::Run { name, capabilities } => {
            let caps = parse_capabilities(&capabilities);
            let node = build_node(&name, &format!("{} agent", name), caps, transport, identity)?;

            if let Some(ref kf) = identity.keyfile {
                println!("Using keyfile: {}", kf.display());
            }
            println!("Agent: {}", node.card.name);
            println!("Pubkey: {}", node.pubkey());
            if identity.encrypt {
                if let Some(ref bundle) = node.card.intro_bundle {
                    println!("Encryption: ENABLED (X25519+ChaCha20-Poly1305)");
                    println!("X25519 pubkey: {}", bundle.agent_pubkey);
                }
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
            let node = build_node("discovery-client", "temporary", vec![], transport, identity)?;
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
            let encrypt_id = IdentityConfig {
                keyfile: identity.keyfile.clone(),
                encrypt: true,
            };
            let node = build_node("bundle-gen", "temporary", vec![], transport, &encrypt_id)?;
            let bundle = node.card.intro_bundle.as_ref().unwrap();
            let json = serde_json::to_string_pretty(bundle)?;
            println!("{}", json);
        }
    }
    Ok(())
}
