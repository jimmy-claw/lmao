use anyhow::Result;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::cli::SessionAction;
use crate::common::{build_node, IdentityConfig};

pub async fn handle(
    action: SessionAction,
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
) -> Result<()> {
    let node = build_node("cli-session", "CLI client", vec![], transport, identity)?;

    match action {
        SessionAction::List => {
            let sessions = node.list_sessions();
            if sessions.is_empty() {
                println!("No active sessions.");
            } else {
                println!("{} session(s):", sessions.len());
                for s in &sessions {
                    println!("  {} peer={} tasks={}", s.id, s.peer, s.task_ids.len());
                }
            }
        }
        SessionAction::Show { id } => match node.get_session(&id) {
            Some(s) => {
                println!("{}", serde_json::to_string_pretty(&s)?);
            }
            None => {
                eprintln!("Session {} not found.", id);
            }
        },
    }

    Ok(())
}
