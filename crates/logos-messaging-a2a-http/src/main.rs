//! A2A-spec HTTP/JSON-RPC server for Logos Messaging agents.
//!
//! Implements the [Google A2A specification](https://google.github.io/A2A/)
//! JSON-RPC interface over HTTP, backed by a `WakuA2ANode` for transport.
//!
//! # Endpoints
//!
//! - `POST /` — JSON-RPC dispatch (`tasks/send`, `tasks/get`)
//! - `GET /.well-known/agent.json` — AgentCard discovery
//!
//! # x402 Payment Middleware
//!
//! When `--payment-enabled` is set, `tasks/send` requires an `X-Payment-Proof`
//! header containing a transaction hash proving on-chain payment.
//!
//! # Usage
//!
//! ```bash
//! # Without payment
//! logos-messaging-a2a-http --waku-url http://localhost:8645 --port 3000
//!
//! # With x402 payment required
//! logos-messaging-a2a-http --waku-url http://localhost:8645 --port 3000 \
//!     --payment-enabled --payment-price 10 \
//!     --payment-token-program LEZprog111 \
//!     --payment-receiving-account LEZacct222
//! ```

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    middleware as axum_mw,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use logos_messaging_a2a_core::{Part, Task, TaskState};
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;

pub mod payment;
use payment::{PaymentConfig, payment_middleware};

// ── CLI ──────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "logos-messaging-a2a-http",
    about = "A2A-spec HTTP server for Logos Messaging agents"
)]
struct Cli {
    /// nwaku REST API URL
    #[arg(long, default_value = "http://localhost:8645")]
    waku_url: String,

    /// HTTP server port
    #[arg(long, default_value_t = 3000)]
    port: u16,

    /// Timeout in seconds for waiting on task responses
    #[arg(long, default_value_t = 30)]
    timeout: u64,

    // ── x402 payment options ─────────────────────────────────────────

    /// Enable x402 payment requirement for tasks/send
    #[arg(long, default_value_t = false)]
    payment_enabled: bool,

    /// Price per task (in smallest token unit)
    #[arg(long, default_value_t = 0)]
    payment_price: u64,

    /// Token program ID (on-chain address)
    #[arg(long, default_value = "")]
    payment_token_program: String,

    /// Receiving account address for payments
    #[arg(long, default_value = "")]
    payment_receiving_account: String,

    /// Token name for display (default: LEZ)
    #[arg(long, default_value = "LEZ")]
    payment_token_name: String,
}

// ── JSON-RPC types ───────────────────────────────────────────────────────

/// JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: serde_json::Value,
}

/// JSON-RPC error object.
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error(id: serde_json::Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }
}

// ── JSON-RPC error codes ─────────────────────────────────────────────────

const PARSE_ERROR: i32 = -32700;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;
/// x402: Payment Required (used by payment middleware).
pub const PAYMENT_REQUIRED: i32 = -32402;

// ── Request/Response types for A2A methods ───────────────────────────────

/// Parameters for `tasks/send`.
#[derive(Debug, Deserialize)]
pub struct TaskSendParams {
    /// Target agent public key (hex)
    pub to: String,
    /// Message text
    pub message: String,
    /// Optional session ID for multi-turn conversations
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Parameters for `tasks/get`.
#[derive(Debug, Deserialize)]
pub struct TaskGetParams {
    /// Task ID to query
    pub task_id: String,
}

/// Serializable task response.
#[derive(Debug, Serialize)]
pub struct TaskResponse {
    pub id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

impl From<&Task> for TaskResponse {
    fn from(task: &Task) -> Self {
        let result_text = task.result.as_ref().map(|m| {
            m.parts
                .iter()
                .map(|p| match p {
                    Part::Text { text } => text.as_str(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
        Self {
            id: task.id.clone(),
            state: format!("{:?}", task.state).to_lowercase(),
            result: result_text,
        }
    }
}

// ── App state ────────────────────────────────────────────────────────────

pub struct AppState {
    pub node: RwLock<WakuA2ANode<LogosMessagingTransport>>,
    pub timeout: Duration,
}

// ── Handlers ─────────────────────────────────────────────────────────────

/// `GET /.well-known/agent.json` — A2A agent card discovery.
pub async fn agent_card(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let node = state.node.read().await;
    Json(serde_json::to_value(&node.card).unwrap_or_default())
}

/// `POST /` — JSON-RPC dispatch.
pub async fn jsonrpc_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Parse the JSON-RPC request
    let req: JsonRpcRequest = match serde_json::from_value(req) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::OK,
                Json(JsonRpcResponse::error(
                    serde_json::Value::Null,
                    PARSE_ERROR,
                    format!("Parse error: {e}"),
                )),
            );
        }
    };

    if req.jsonrpc != "2.0" {
        return (
            StatusCode::OK,
            Json(JsonRpcResponse::error(
                req.id,
                PARSE_ERROR,
                "Invalid JSON-RPC version (expected \"2.0\")",
            )),
        );
    }

    let response = match req.method.as_str() {
        "tasks/send" => handle_tasks_send(&state, req.id.clone(), req.params).await,
        "tasks/get" => handle_tasks_get(&state, req.id.clone(), req.params).await,
        "agent/authenticatedExtendedCard" => {
            let node = state.node.read().await;
            let card = serde_json::to_value(&node.card).unwrap_or_default();
            JsonRpcResponse::success(req.id, card)
        }
        _ => JsonRpcResponse::error(
            req.id,
            METHOD_NOT_FOUND,
            format!("Method '{}' not found", req.method),
        ),
    };

    (StatusCode::OK, Json(response))
}

async fn handle_tasks_send(
    state: &AppState,
    id: serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params: TaskSendParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    let node = state.node.read().await;

    // Send the task (with optional session)
    let task = if let Some(ref sid) = params.session_id {
        match node.send_in_session(sid, &params.message).await {
            Ok(t) => t,
            Err(e) => {
                return JsonRpcResponse::error(id, INTERNAL_ERROR, format!("Send failed: {e}"));
            }
        }
    } else {
        match node.send_text(&params.to, &params.message).await {
            Ok(t) => t,
            Err(e) => {
                return JsonRpcResponse::error(id, INTERNAL_ERROR, format!("Send failed: {e}"));
            }
        }
    };

    let task_id = task.id.clone();

    // Wait for response with timeout
    let deadline = tokio::time::Instant::now() + state.timeout;

    loop {
        if tokio::time::Instant::now() > deadline {
            return JsonRpcResponse::success(
                id,
                serde_json::json!({
                    "id": task_id,
                    "state": "submitted",
                    "result": null,
                    "timeout": true
                }),
            );
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        let tasks = match node.poll_tasks().await {
            Ok(t) => t,
            Err(_) => continue,
        };

        if let Some(response) = tasks.iter().find(|t| t.id == task_id) {
            match response.state {
                TaskState::Completed | TaskState::Failed => {
                    let resp = TaskResponse::from(response);
                    return JsonRpcResponse::success(
                        id,
                        serde_json::to_value(resp).unwrap_or_default(),
                    );
                }
                _ => continue,
            }
        }
    }
}

async fn handle_tasks_get(
    state: &AppState,
    id: serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params: TaskGetParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    let node = state.node.read().await;
    let tasks = match node.poll_tasks().await {
        Ok(t) => t,
        Err(e) => {
            return JsonRpcResponse::error(id, INTERNAL_ERROR, format!("Poll failed: {e}"));
        }
    };

    match tasks.iter().find(|t| t.id == params.task_id) {
        Some(task) => {
            let resp = TaskResponse::from(task);
            JsonRpcResponse::success(id, serde_json::to_value(resp).unwrap_or_default())
        }
        None => JsonRpcResponse::error(
            id,
            INVALID_PARAMS,
            format!("Task '{}' not found", params.task_id),
        ),
    }
}

/// Build the axum router with optional x402 payment middleware.
pub fn build_router(state: Arc<AppState>, payment_config: Arc<PaymentConfig>) -> Router {
    let payment_cfg = payment_config.clone();
    Router::new()
        .route("/.well-known/agent.json", get(agent_card))
        .route("/", post(jsonrpc_handler))
        .layer(axum_mw::from_fn(move |headers, req, next| {
            let cfg = payment_cfg.clone();
            payment_middleware(headers, cfg, req, next)
        }))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    let payment_config = Arc::new(PaymentConfig {
        enabled: cli.payment_enabled,
        price_per_task: cli.payment_price,
        token_program_id: cli.payment_token_program.clone(),
        receiving_account: cli.payment_receiving_account.clone(),
        token_name: cli.payment_token_name.clone(),
    });

    if payment_config.enabled {
        tracing::info!(
            "x402 payment enabled: {} {} per task → {}",
            payment_config.price_per_task,
            payment_config.token_name,
            payment_config.receiving_account
        );
    }

    tracing::info!(
        "Starting A2A HTTP server (waku: {}, port: {}, timeout: {}s)",
        cli.waku_url,
        cli.port,
        cli.timeout
    );

    let transport = LogosMessagingTransport::new(&cli.waku_url);
    let node = WakuA2ANode::new(
        "http-server",
        "A2A HTTP/JSON-RPC server — standard HTTP interface to Logos agents",
        vec!["http-server".into()],
        transport,
    );

    // Announce on network
    if let Err(e) = node.announce().await {
        tracing::warn!("Failed to announce on network: {e}");
    }

    let state = Arc::new(AppState {
        node: RwLock::new(node),
        timeout: Duration::from_secs(cli.timeout),
    });

    let app = build_router(state, payment_config);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cli.port)).await?;
    tracing::info!("Listening on 0.0.0.0:{}", cli.port);
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonrpc_response_success_serialization() {
        let resp = JsonRpcResponse::success(
            serde_json::json!(1),
            serde_json::json!({"status": "ok"}),
        );
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["result"]["status"], "ok");
        assert!(json.get("error").is_none());
        assert_eq!(json["id"], 1);
    }

    #[test]
    fn jsonrpc_response_error_serialization() {
        let resp = JsonRpcResponse::error(
            serde_json::json!("abc"),
            METHOD_NOT_FOUND,
            "not found",
        );
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert!(json.get("result").is_none());
        assert_eq!(json["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(json["error"]["message"], "not found");
        assert_eq!(json["id"], "abc");
    }

    #[test]
    fn task_response_from_completed_task() {
        let task = Task::new("0xfrom", "0xto", "hello");
        let mut task = task;
        task.state = TaskState::Completed;
        task.result = Some(logos_messaging_a2a_core::Message {
            role: "agent".into(),
            parts: vec![Part::Text { text: "world".into() }],
        });
        let resp = TaskResponse::from(&task);
        assert_eq!(resp.state, "completed");
        assert_eq!(resp.result, Some("world".into()));
    }

    #[test]
    fn task_response_from_submitted_task() {
        let task = Task::new("0xfrom", "0xto", "hello");
        let resp = TaskResponse::from(&task);
        assert_eq!(resp.state, "submitted");
        assert_eq!(resp.result, None);
    }

    #[test]
    fn parse_task_send_params() {
        let json = serde_json::json!({
            "to": "0xabc",
            "message": "hello",
            "session_id": "sess-1"
        });
        let params: TaskSendParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.to, "0xabc");
        assert_eq!(params.message, "hello");
        assert_eq!(params.session_id, Some("sess-1".into()));
    }

    #[test]
    fn parse_task_send_params_no_session() {
        let json = serde_json::json!({
            "to": "0xabc",
            "message": "hello"
        });
        let params: TaskSendParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.session_id, None);
    }

    #[test]
    fn parse_task_get_params() {
        let json = serde_json::json!({"task_id": "task-123"});
        let params: TaskGetParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.task_id, "task-123");
    }

    #[test]
    fn payment_required_code() {
        assert_eq!(PAYMENT_REQUIRED, -32402);
    }
}
