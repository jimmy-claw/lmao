use anyhow::Result;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

use crate::cli::SessionAction;
use crate::common::{build_node, IdentityConfig};

pub async fn handle(
    action: SessionAction,
    transport: LogosMessagingTransport,
    identity: &IdentityConfig,
    json: bool,
) -> Result<()> {
    let node = build_node("cli-session", "CLI client", vec![], transport, identity)?;

    match action {
        SessionAction::List => {
            let sessions = node.list_sessions();
            if json {
                let items: Vec<_> = sessions
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id,
                            "peer": s.peer,
                            "task_count": s.task_ids.len(),
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({ "sessions": items }))?
                );
            } else if sessions.is_empty() {
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
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "error": "not_found",
                            "session_id": id,
                        }))?
                    );
                } else {
                    eprintln!("Session {} not found.", id);
                }
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_list_json_output_is_parseable() {
        let items = vec![serde_json::json!({
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "peer": "02aabbcc",
            "task_count": 3,
        })];
        let output = serde_json::to_string(&serde_json::json!({ "sessions": items })).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let sessions = parsed["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["task_count"], 3);
        assert!(sessions[0]["id"].is_string());
    }

    #[test]
    fn session_not_found_json_output_is_parseable() {
        let output = serde_json::to_string(&serde_json::json!({
            "error": "not_found",
            "session_id": "missing-id",
        }))
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["error"], "not_found");
        assert_eq!(parsed["session_id"], "missing-id");
    }
}
