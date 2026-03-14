//! Integration tests for the MCP bridge crate.
//!
//! These tests exercise the full bridge lifecycle — discovery, presence, task
//! send/receive, tool listing, and error handling — using [`InMemoryTransport`]
//! so no external nwaku node is required.

use logos_messaging_a2a_core::{topics, A2AEnvelope, PresenceAnnouncement, TaskState};
use logos_messaging_a2a_mcp::{
    format_peer_entry, result_text, GetAgentStatusInput, LogosA2ABridge, SendToAgentInput,
};
use logos_messaging_a2a_node::presence::PeerInfo;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::memory::InMemoryTransport;
use logos_messaging_a2a_transport::sds::ChannelConfig;
use logos_messaging_a2a_transport::Transport;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::ServerHandler;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Create a [`LogosA2ABridge`] backed by the given in-memory transport.
fn make_bridge(transport: InMemoryTransport) -> LogosA2ABridge<InMemoryTransport> {
    let node = WakuA2ANode::new(
        "mcp-bridge",
        "MCP bridge for integration tests",
        vec!["mcp-bridge".into()],
        transport,
    );
    LogosA2ABridge::from_node(node, 30)
}

/// Create a bridge with fire-and-forget SDS config and a custom timeout.
/// This avoids blocking on ACK waits in tests.
fn make_fast_bridge(
    transport: InMemoryTransport,
    timeout_secs: u64,
) -> LogosA2ABridge<InMemoryTransport> {
    let node = WakuA2ANode::with_config(
        "mcp-bridge",
        "MCP bridge for integration tests",
        vec!["mcp-bridge".into()],
        transport,
        ChannelConfig::fire_and_forget(),
    );
    LogosA2ABridge::from_node(node, timeout_secs)
}

/// Create a WakuA2ANode with fire-and-forget SDS config for fast tests.
fn make_fast_agent(
    name: &str,
    desc: &str,
    caps: Vec<String>,
    transport: InMemoryTransport,
) -> WakuA2ANode<InMemoryTransport> {
    WakuA2ANode::with_config(
        name,
        desc,
        caps,
        transport,
        ChannelConfig::fire_and_forget(),
    )
}

// ═══════════════════════════════════════════════════════════════════════════
//  1. MCP Tool Listing
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tool_list_returns_all_five_tools() {
    let bridge = make_bridge(InMemoryTransport::new());
    let tools = bridge.tool_router.list_all();

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"discover_agents"),
        "missing discover_agents"
    );
    assert!(names.contains(&"send_to_agent"), "missing send_to_agent");
    assert!(
        names.contains(&"list_cached_agents"),
        "missing list_cached_agents"
    );
    assert!(
        names.contains(&"discover_agents_presence"),
        "missing discover_agents_presence"
    );
    assert!(
        names.contains(&"get_agent_status"),
        "missing get_agent_status"
    );
    assert_eq!(tools.len(), 5, "unexpected number of tools");
}

#[tokio::test]
async fn tool_list_every_tool_has_description() {
    let bridge = make_bridge(InMemoryTransport::new());
    let tools = bridge.tool_router.list_all();

    for tool in &tools {
        assert!(
            tool.description.as_ref().map_or(false, |d| !d.is_empty()),
            "tool '{}' missing description",
            tool.name
        );
    }
}

#[tokio::test]
async fn server_info_exposes_tools_capability() {
    let bridge = make_bridge(InMemoryTransport::new());
    let info = bridge.get_info();

    assert!(info.capabilities.tools.is_some());
    assert!(info.instructions.is_some());
    let instructions = info.instructions.unwrap();
    assert!(instructions.contains("discover_agents"));
    assert!(instructions.contains("send_to_agent"));
}

// ═══════════════════════════════════════════════════════════════════════════
//  2. Agent Discovery (legacy broadcast)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn discover_then_list_cached() {
    let transport = InMemoryTransport::new();

    // Two agents announce on the shared transport.
    let agent_a = WakuA2ANode::new(
        "summarizer",
        "Summarizes documents",
        vec!["summarize".into()],
        transport.clone(),
    );
    let agent_b = WakuA2ANode::new(
        "translator",
        "Translates text",
        vec!["translate".into()],
        transport.clone(),
    );
    agent_a.announce().await.unwrap();
    agent_b.announce().await.unwrap();

    let bridge = make_bridge(transport);

    // discover_agents should find both.
    let result = bridge.discover_agents().await.unwrap();
    let text = result_text(&result);
    assert!(text.contains("Found 2 agent(s)"));
    assert!(text.contains("summarizer"));
    assert!(text.contains("translator"));

    // list_cached_agents should reflect the cache.
    let cached = bridge.list_cached_agents().await.unwrap();
    let cached_text = result_text(&cached);
    assert!(cached_text.contains("summarizer"));
    assert!(cached_text.contains("translator"));
}

#[tokio::test]
async fn discover_filters_out_bridge_self() {
    let transport = InMemoryTransport::new();

    // Bridge announces itself + an external agent.
    let bridge = make_bridge(transport.clone());
    {
        let node = bridge.node.read().await;
        node.announce().await.unwrap();
    }
    let external = WakuA2ANode::new(
        "external-agent",
        "External",
        vec!["ext".into()],
        transport.clone(),
    );
    external.announce().await.unwrap();

    let result = bridge.discover_agents().await.unwrap();
    let text = result_text(&result);

    // The bridge's own card should be filtered by discover().
    assert!(text.contains("external-agent"));
    // Only the external agent should appear.
    assert!(text.contains("Found 1 agent(s)"));
}

#[tokio::test]
async fn discover_populates_cache_for_send() {
    let transport = InMemoryTransport::new();

    let agent = WakuA2ANode::new(
        "echo-bot",
        "Echo bot",
        vec!["echo".into()],
        transport.clone(),
    );
    agent.announce().await.unwrap();

    let bridge = make_bridge(transport);

    // Before discovery, cache is empty.
    let cached = bridge.list_cached_agents().await.unwrap();
    assert!(result_text(&cached).contains("No cached agents"));

    // After discovery, cache is populated.
    bridge.discover_agents().await.unwrap();
    let cached = bridge.list_cached_agents().await.unwrap();
    assert!(result_text(&cached).contains("echo-bot"));
}

// ═══════════════════════════════════════════════════════════════════════════
//  3. Send-to-Agent with Mock Transport
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn send_to_agent_receives_completed_response() {
    let transport = InMemoryTransport::new();

    // Use fire-and-forget config so send_text doesn't block on ACK.
    let echo_agent = make_fast_agent(
        "echo",
        "Echoes messages",
        vec!["echo".into()],
        transport.clone(),
    );
    echo_agent.announce().await.unwrap();

    let bridge = make_fast_bridge(transport.clone(), 10);
    bridge.discover_agents().await.unwrap();

    let bridge_pubkey = {
        let node = bridge.node.read().await;
        node.pubkey().to_string()
    };

    // Spawn the echo agent to listen and respond.
    let echo_handle = tokio::spawn({
        let echo = echo_agent;
        let bridge_pk = bridge_pubkey;
        async move {
            for _ in 0..50 {
                let tasks = echo.poll_tasks().await.unwrap();
                for task in &tasks {
                    if task.from == bridge_pk {
                        echo.respond(task, &format!("Echo: {}", task.text().unwrap_or("")))
                            .await
                            .unwrap();
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    });

    let result = bridge
        .send_to_agent(Parameters(SendToAgentInput {
            agent_name: "echo".to_string(),
            message: "Hello, echo!".to_string(),
        }))
        .await
        .unwrap();

    let text = result_text(&result);
    assert!(
        text.contains("Echo: Hello, echo!"),
        "Expected echo response, got: {text}"
    );
    assert!(text.contains("Response from 'echo'"));

    echo_handle.await.unwrap();
}

#[tokio::test]
async fn send_to_agent_receives_failed_response() {
    let transport = InMemoryTransport::new();

    let fail_agent = make_fast_agent(
        "fail-bot",
        "Always fails",
        vec!["fail".into()],
        transport.clone(),
    );
    fail_agent.announce().await.unwrap();

    let bridge = make_fast_bridge(transport.clone(), 10);
    bridge.discover_agents().await.unwrap();

    let bridge_pubkey = {
        let node = bridge.node.read().await;
        node.pubkey().to_string()
    };

    // Spawn agent that responds with a failed task via raw transport.
    let fail_handle = tokio::spawn({
        let agent = fail_agent;
        let bridge_pk = bridge_pubkey;
        let t = transport.clone();
        async move {
            for _ in 0..50 {
                let tasks = agent.poll_tasks().await.unwrap();
                for task in &tasks {
                    if task.from == bridge_pk {
                        let mut failed = task.respond("error");
                        failed.state = TaskState::Failed;
                        let envelope = A2AEnvelope::Task(failed.clone());
                        let payload = serde_json::to_vec(&envelope).unwrap();
                        let topic = topics::task_topic(&failed.to);
                        t.publish(&topic, &payload).await.unwrap();
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    });

    let result = bridge
        .send_to_agent(Parameters(SendToAgentInput {
            agent_name: "fail-bot".to_string(),
            message: "do something".to_string(),
        }))
        .await
        .unwrap();

    let text = result_text(&result);
    assert!(
        text.contains("task failed"),
        "Expected failure message, got: {text}"
    );

    fail_handle.await.unwrap();
}

#[tokio::test]
async fn send_to_agent_timeout() {
    let transport = InMemoryTransport::new();

    // Agent that never responds.
    let silent = make_fast_agent(
        "silent",
        "Never responds",
        vec!["silent".into()],
        transport.clone(),
    );
    silent.announce().await.unwrap();

    // Use a very short timeout so the test doesn't hang.
    let bridge = make_fast_bridge(transport, 1);
    bridge.discover_agents().await.unwrap();

    let result = bridge
        .send_to_agent(Parameters(SendToAgentInput {
            agent_name: "silent".to_string(),
            message: "anyone there?".to_string(),
        }))
        .await
        .unwrap();

    let text = result_text(&result);
    assert!(
        text.contains("Timeout"),
        "Expected timeout message, got: {text}"
    );
    assert!(text.contains("silent"));
}

// ═══════════════════════════════════════════════════════════════════════════
//  4. Presence Monitoring
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn presence_discover_and_status_check() {
    let transport = InMemoryTransport::new();

    let agent = WakuA2ANode::new(
        "live-agent",
        "Always online",
        vec!["chat".into(), "search".into()],
        transport.clone(),
    );
    let agent_id = agent.pubkey().to_string();
    agent.announce_presence().await.unwrap();

    let bridge = make_bridge(transport);

    // Presence discovery should find the agent.
    let result = bridge.discover_agents_presence().await.unwrap();
    let text = result_text(&result);
    assert!(text.contains("Found 1 live agent(s)"));
    assert!(text.contains("live-agent"));
    assert!(text.contains("chat, search"));

    // Status check should show ONLINE.
    let status = bridge
        .get_agent_status(Parameters(GetAgentStatusInput {
            agent_id: agent_id.clone(),
        }))
        .await
        .unwrap();
    let status_text = result_text(&status);
    assert!(status_text.contains("ONLINE"));
    assert!(status_text.contains("live-agent"));
}

#[tokio::test]
async fn presence_excludes_bridge_self_announcements() {
    let transport = InMemoryTransport::new();
    let bridge = make_bridge(transport.clone());

    // Bridge announces its own presence.
    {
        let node = bridge.node.read().await;
        node.announce_presence().await.unwrap();
    }

    // Also have an external agent.
    let external = WakuA2ANode::new(
        "ext-agent",
        "External",
        vec!["ext".into()],
        transport.clone(),
    );
    external.announce_presence().await.unwrap();

    let result = bridge.discover_agents_presence().await.unwrap();
    let text = result_text(&result);

    // Only the external agent should appear.
    assert!(text.contains("ext-agent"));
    assert!(text.contains("Found 1 live agent(s)"));
}

#[tokio::test]
async fn presence_multiple_agents_numbered() {
    let transport = InMemoryTransport::new();

    for (name, cap) in [
        ("agent-1", "summarize"),
        ("agent-2", "translate"),
        ("agent-3", "code"),
    ] {
        let node = WakuA2ANode::new(
            name,
            &format!("{name} desc"),
            vec![cap.into()],
            transport.clone(),
        );
        node.announce_presence().await.unwrap();
    }

    let bridge = make_bridge(transport);
    let result = bridge.discover_agents_presence().await.unwrap();
    let text = result_text(&result);

    assert!(text.contains("Found 3 live agent(s)"));
    // Verify numbered format.
    assert!(text.contains("1. **"));
    assert!(text.contains("2. **"));
    assert!(text.contains("3. **"));
}

#[tokio::test]
async fn get_agent_status_unknown_agent_is_offline() {
    let bridge = make_bridge(InMemoryTransport::new());

    let result = bridge
        .get_agent_status(Parameters(GetAgentStatusInput {
            agent_id: "0000000000000000deadbeef".to_string(),
        }))
        .await
        .unwrap();

    let text = result_text(&result);
    assert!(text.contains("OFFLINE or unknown"));
}

#[tokio::test]
async fn get_agent_status_shows_all_fields() {
    let transport = InMemoryTransport::new();

    let agent = WakuA2ANode::new(
        "full-info",
        "Agent with full info",
        vec!["a".into(), "b".into(), "c".into()],
        transport.clone(),
    );
    let agent_id = agent.pubkey().to_string();
    agent.announce_presence().await.unwrap();

    let bridge = make_bridge(transport);
    let result = bridge
        .get_agent_status(Parameters(GetAgentStatusInput {
            agent_id: agent_id.clone(),
        }))
        .await
        .unwrap();
    let text = result_text(&result);

    assert!(text.contains("ONLINE"));
    assert!(text.contains("full-info"));
    assert!(text.contains("a, b, c"));
    assert!(text.contains("Waku topic:"));
    assert!(text.contains("TTL:"));
    assert!(text.contains("last seen"));
    assert!(text.contains(&agent_id[..16]));
}

// ═══════════════════════════════════════════════════════════════════════════
//  5. Error Handling
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn send_to_agent_unknown_returns_invalid_params() {
    let bridge = make_bridge(InMemoryTransport::new());

    let err = bridge
        .send_to_agent(Parameters(SendToAgentInput {
            agent_name: "nonexistent-agent".to_string(),
            message: "hello".to_string(),
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("nonexistent-agent"));
    assert!(err.message.contains("discover_agents"));
}

#[tokio::test]
async fn send_to_agent_empty_cache_returns_error() {
    let bridge = make_bridge(InMemoryTransport::new());

    // Without calling discover_agents first, cache is empty.
    let err = bridge
        .send_to_agent(Parameters(SendToAgentInput {
            agent_name: "any-agent".to_string(),
            message: "hello".to_string(),
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn list_cached_agents_empty_says_no_cached() {
    let bridge = make_bridge(InMemoryTransport::new());
    let result = bridge.list_cached_agents().await.unwrap();
    assert!(result_text(&result).contains("No cached agents"));
}

#[tokio::test]
async fn discover_on_empty_network() {
    let bridge = make_bridge(InMemoryTransport::new());
    let result = bridge.discover_agents().await.unwrap();
    assert!(result_text(&result).contains("No agents found"));
}

#[tokio::test]
async fn presence_rejects_unsigned_announcements() {
    let transport = InMemoryTransport::new();

    // Inject an unsigned presence announcement directly onto the wire.
    let unsigned = PresenceAnnouncement {
        agent_id: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef00".to_string(),
        name: "malicious".to_string(),
        capabilities: vec!["evil".into()],
        waku_topic: "/fake/topic".to_string(),
        ttl_secs: 300,
        signature: None,
    };
    let envelope = A2AEnvelope::Presence(unsigned);
    let payload = serde_json::to_vec(&envelope).unwrap();
    transport.publish(topics::PRESENCE, &payload).await.unwrap();

    let bridge = make_bridge(transport);
    let result = bridge.discover_agents_presence().await.unwrap();
    let text = result_text(&result);

    assert!(
        text.contains("No agents currently online"),
        "unsigned announcement should be rejected"
    );
}

#[tokio::test]
async fn presence_rejects_malformed_envelope() {
    let transport = InMemoryTransport::new();

    // Inject garbage data on the presence topic.
    transport
        .publish(topics::PRESENCE, b"not valid json")
        .await
        .unwrap();

    let bridge = make_bridge(transport);
    // Should not panic; malformed data is silently ignored.
    let result = bridge.discover_agents_presence().await.unwrap();
    let text = result_text(&result);
    assert!(text.contains("No agents currently online"));
}

// ═══════════════════════════════════════════════════════════════════════════
//  6. JSON-RPC Request/Response Formatting
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn send_to_agent_input_deserializes_from_json() {
    let json = r#"{"agent_name":"echo-bot","message":"Summarize this document"}"#;
    let input: SendToAgentInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.agent_name, "echo-bot");
    assert_eq!(input.message, "Summarize this document");
}

#[test]
fn send_to_agent_input_deserializes_with_whitespace() {
    let json = r#"{
        "agent_name": "echo-bot",
        "message": "Summarize this document"
    }"#;
    let input: SendToAgentInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.agent_name, "echo-bot");
    assert_eq!(input.message, "Summarize this document");
}

#[test]
fn get_agent_status_input_json_roundtrip() {
    let json =
        r#"{"agent_id":"02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"}"#;
    let input: GetAgentStatusInput = serde_json::from_str(json).unwrap();
    assert_eq!(
        input.agent_id,
        "02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
    );
}

#[test]
fn send_to_agent_input_rejects_incomplete_json() {
    // Missing message field.
    assert!(serde_json::from_str::<SendToAgentInput>(r#"{"agent_name":"x"}"#).is_err());
    // Missing agent_name field.
    assert!(serde_json::from_str::<SendToAgentInput>(r#"{"message":"hi"}"#).is_err());
    // Empty object.
    assert!(serde_json::from_str::<SendToAgentInput>(r#"{}"#).is_err());
}

#[test]
fn get_agent_status_input_rejects_empty_json() {
    assert!(serde_json::from_str::<GetAgentStatusInput>(r#"{}"#).is_err());
}

#[test]
fn send_to_agent_input_ignores_extra_fields() {
    let json = r#"{"agent_name":"a","message":"b","extra":"ignored","nested":{"x":1}}"#;
    let input: SendToAgentInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.agent_name, "a");
    assert_eq!(input.message, "b");
}

#[tokio::test]
async fn discover_agents_result_format_is_well_structured() {
    let transport = InMemoryTransport::new();

    let agent = WakuA2ANode::new(
        "format-agent",
        "Agent for format testing",
        vec!["cap-a".into(), "cap-b".into()],
        transport.clone(),
    );
    agent.announce().await.unwrap();

    let bridge = make_bridge(transport);
    let result = bridge.discover_agents().await.unwrap();
    let text = result_text(&result);

    // Verify structured output format.
    assert!(text.starts_with("Found 1 agent(s):"));
    assert!(text.contains("1. **format-agent**"));
    assert!(text.contains("(v0.1.0)"));
    assert!(text.contains("Agent for format testing"));
    assert!(text.contains("Capabilities: [cap-a, cap-b]"));
    assert!(text.contains("Public key:"));
    assert!(text.contains("..."));
}

#[test]
fn format_peer_entry_produces_expected_layout() {
    let info = PeerInfo {
        name: "peer-x".to_string(),
        capabilities: vec!["search".to_string(), "index".to_string()],
        waku_topic: "/waku-a2a/1/task/abcdef/proto".to_string(),
        ttl_secs: 600,
        last_seen: 1_700_000_000,
    };

    let output = format_peer_entry(3, "abcdef1234567890fedcba", &info);
    assert!(output.contains("3. **peer-x**"));
    assert!(output.contains("[search, index]"));
    assert!(output.contains("Agent ID: abcdef1234567890..."));
    assert!(output.contains("Topic: /waku-a2a/1/task/abcdef/proto"));
    assert!(output.contains("TTL: 600s"));
}

// ═══════════════════════════════════════════════════════════════════════════
//  7. Bridge Cloning & Shared State
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cloned_bridge_shares_agent_cache() {
    let transport = InMemoryTransport::new();

    let agent = WakuA2ANode::new("shared", "Shared", vec!["s".into()], transport.clone());
    agent.announce().await.unwrap();

    let bridge = make_bridge(transport);
    let clone = bridge.clone();

    // Discovery on the original should be visible on the clone.
    bridge.discover_agents().await.unwrap();

    let cached = clone.list_cached_agents().await.unwrap();
    assert!(result_text(&cached).contains("shared"));
}

#[tokio::test]
async fn cloned_bridge_shares_node() {
    let transport = InMemoryTransport::new();
    let bridge = make_bridge(transport);
    let clone = bridge.clone();

    // Both should reference the same underlying node (same pubkey).
    let pk1 = bridge.node.read().await.pubkey().to_string();
    let pk2 = clone.node.read().await.pubkey().to_string();
    assert_eq!(pk1, pk2);
}

// ═══════════════════════════════════════════════════════════════════════════
//  8. Discovery-then-Send Lifecycle
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn full_lifecycle_discover_send_receive() {
    let transport = InMemoryTransport::new();

    // Use fire-and-forget config for fast tests.
    let responder = make_fast_agent(
        "responder",
        "Responds to queries",
        vec!["query".into()],
        transport.clone(),
    );
    responder.announce().await.unwrap();
    responder.announce_presence().await.unwrap();

    let bridge = make_fast_bridge(transport.clone(), 10);

    // Step 1: Legacy discovery.
    let discover_result = bridge.discover_agents().await.unwrap();
    assert!(result_text(&discover_result).contains("responder"));

    // Step 2: Presence discovery sees the same agent.
    let presence_result = bridge.discover_agents_presence().await.unwrap();
    assert!(result_text(&presence_result).contains("responder"));

    // Step 3: Check status.
    let agent_id = responder.pubkey().to_string();
    let status = bridge
        .get_agent_status(Parameters(GetAgentStatusInput {
            agent_id: agent_id.clone(),
        }))
        .await
        .unwrap();
    assert!(result_text(&status).contains("ONLINE"));

    // Step 4: Send a message and have the responder reply.
    let bridge_pubkey = bridge.node.read().await.pubkey().to_string();
    let responder_handle = tokio::spawn({
        let resp = responder;
        async move {
            for _ in 0..50 {
                let tasks = resp.poll_tasks().await.unwrap();
                for task in &tasks {
                    if task.from == bridge_pubkey {
                        resp.respond(task, "lifecycle response").await.unwrap();
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    });

    let send_result = bridge
        .send_to_agent(Parameters(SendToAgentInput {
            agent_name: "responder".to_string(),
            message: "lifecycle test".to_string(),
        }))
        .await
        .unwrap();

    assert!(result_text(&send_result).contains("lifecycle response"));
    responder_handle.await.unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
//  9. Edge Cases
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn discover_agents_rediscovery_replaces_cache() {
    let transport = InMemoryTransport::new();

    let a = WakuA2ANode::new("agent-a", "A", vec!["a".into()], transport.clone());
    a.announce().await.unwrap();

    let bridge = make_bridge(transport.clone());

    // First discovery.
    bridge.discover_agents().await.unwrap();
    assert_eq!(bridge.agents.read().await.len(), 1);

    // Announce another agent.
    let b = WakuA2ANode::new("agent-b", "B", vec!["b".into()], transport.clone());
    b.announce().await.unwrap();

    // Second discovery replaces the cache (InMemoryTransport replays all history).
    bridge.discover_agents().await.unwrap();
    let agents = bridge.agents.read().await;
    assert_eq!(agents.len(), 2);
}

#[tokio::test]
async fn send_to_unicode_message() {
    let transport = InMemoryTransport::new();

    let agent = make_fast_agent(
        "unicode-bot",
        "Handles unicode",
        vec!["text".into()],
        transport.clone(),
    );
    agent.announce().await.unwrap();

    let bridge = make_fast_bridge(transport.clone(), 10);
    bridge.discover_agents().await.unwrap();

    let bridge_pubkey = bridge.node.read().await.pubkey().to_string();
    let agent_handle = tokio::spawn({
        let a = agent;
        async move {
            for _ in 0..50 {
                let tasks = a.poll_tasks().await.unwrap();
                for task in &tasks {
                    if task.from == bridge_pubkey {
                        a.respond(task, "Got it!").await.unwrap();
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    });

    let result = bridge
        .send_to_agent(Parameters(SendToAgentInput {
            agent_name: "unicode-bot".to_string(),
            message: "Hello! Bonjour! Hola!".to_string(),
        }))
        .await
        .unwrap();

    assert!(result_text(&result).contains("Got it!"));
    agent_handle.await.unwrap();
}

#[tokio::test]
async fn presence_no_peers_returns_helpful_message() {
    let bridge = make_bridge(InMemoryTransport::new());
    let result = bridge.discover_agents_presence().await.unwrap();
    let text = result_text(&result);

    assert!(text.contains("No agents currently online"));
    assert!(text.contains("TTL may have expired"));
}

#[test]
fn format_peer_entry_with_empty_capabilities() {
    let info = PeerInfo {
        name: "no-caps".to_string(),
        capabilities: vec![],
        waku_topic: "/topic".to_string(),
        ttl_secs: 60,
        last_seen: 0,
    };

    let output = format_peer_entry(1, "abcdef1234567890ff", &info);
    assert!(output.contains("1. **no-caps** — []"));
}

#[test]
fn format_peer_entry_short_id_does_not_panic() {
    let info = PeerInfo {
        name: "short".to_string(),
        capabilities: vec!["x".to_string()],
        waku_topic: "/t".to_string(),
        ttl_secs: 10,
        last_seen: 0,
    };

    // Agent ID with fewer than 16 characters should be handled gracefully.
    let output = format_peer_entry(1, "ab", &info);
    assert!(output.contains("ab..."));
}
