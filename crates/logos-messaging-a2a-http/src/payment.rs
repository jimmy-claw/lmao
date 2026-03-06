//! x402 Payment Required middleware for A2A HTTP server.
//!
//! When enabled, the middleware intercepts `tasks/send` JSON-RPC calls and
//! requires an `X-Payment-Proof` header containing a transaction hash that
//! proves on-chain payment before the task is dispatched.
//!
//! # References
//! - x402 spec: <https://x402.org>
//! - Issue: <https://github.com/jimmy-claw/lmao/issues/38>

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Configuration for x402 payment requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentConfig {
    /// Whether payment is required for tasks.
    pub enabled: bool,
    /// Price per task in the smallest token unit (e.g. lamports).
    pub price_per_task: u64,
    /// Token program ID (on-chain address).
    pub token_program_id: String,
    /// Receiving account address.
    pub receiving_account: String,
    /// Human-readable token name for error messages.
    #[serde(default = "default_token_name")]
    pub token_name: String,
}

fn default_token_name() -> String {
    "LEZ".into()
}

impl Default for PaymentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            price_per_task: 0,
            token_program_id: String::new(),
            receiving_account: String::new(),
            token_name: default_token_name(),
        }
    }
}

/// x402 payment details returned in 402 responses.
#[derive(Debug, Serialize)]
pub struct PaymentDetails {
    /// Amount required.
    pub amount: u64,
    /// Token name.
    pub token: String,
    /// Token program address.
    pub token_program_id: String,
    /// Account to send payment to.
    pub receiving_account: String,
    /// Description.
    pub description: String,
}

/// Extracts the JSON-RPC method from the request body, if present.
async fn extract_method(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Peek {
        method: Option<String>,
    }
    serde_json::from_slice::<Peek>(body).ok()?.method
}

/// x402 payment middleware.
///
/// If payment is enabled and the request is a `tasks/send` call without a
/// valid `X-Payment-Proof` header, returns HTTP 402 with payment details.
pub async fn payment_middleware(
    headers: HeaderMap,
    config: Arc<PaymentConfig>,
    request: Request,
    next: Next,
) -> Response {
    if !config.enabled {
        return next.run(request).await;
    }

    // Only gate POST requests (JSON-RPC endpoint)
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }

    // Buffer the body to peek at the method
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
        }
    };

    // Only require payment for tasks/send
    let method = extract_method(&bytes).await;
    let requires_payment = method.as_deref() == Some("tasks/send");

    if !requires_payment {
        let request = Request::from_parts(parts, Body::from(bytes));
        return next.run(request).await;
    }

    // Check for payment proof header
    let payment_proof = headers
        .get("x-payment-proof")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    match payment_proof {
        Some(tx_hash) if !tx_hash.is_empty() => {
            // TODO: Verify tx_hash on-chain (LEZ token transfer to receiving_account
            // for >= price_per_task). For now, accept any non-empty proof.
            tracing::info!(tx_hash = %tx_hash, "Payment proof accepted (verification pending)");
            let request = Request::from_parts(parts, Body::from(bytes));
            next.run(request).await
        }
        _ => {
            let details = PaymentDetails {
                amount: config.price_per_task,
                token: config.token_name.clone(),
                token_program_id: config.token_program_id.clone(),
                receiving_account: config.receiving_account.clone(),
                description: format!(
                    "Payment of {} {} required per task",
                    config.price_per_task, config.token_name
                ),
            };
            (StatusCode::PAYMENT_REQUIRED, Json(details)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payment_config_default_disabled() {
        let config = PaymentConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.price_per_task, 0);
        assert_eq!(config.token_name, "LEZ");
    }

    #[test]
    fn payment_config_deserialize() {
        let json = serde_json::json!({
            "enabled": true,
            "price_per_task": 10,
            "token_program_id": "LEZprog111",
            "receiving_account": "LEZacct222"
        });
        let config: PaymentConfig = serde_json::from_value(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.price_per_task, 10);
        assert_eq!(config.token_name, "LEZ");
    }

    #[test]
    fn payment_details_serialization() {
        let details = PaymentDetails {
            amount: 10,
            token: "LEZ".into(),
            token_program_id: "prog".into(),
            receiving_account: "acct".into(),
            description: "Pay up".into(),
        };
        let json = serde_json::to_value(&details).unwrap();
        assert_eq!(json["amount"], 10);
        assert_eq!(json["token"], "LEZ");
    }

    #[tokio::test]
    async fn extract_method_tasks_send() {
        let body = br#"{"jsonrpc":"2.0","method":"tasks/send","params":{},"id":1}"#;
        assert_eq!(extract_method(body).await, Some("tasks/send".into()));
    }

    #[tokio::test]
    async fn extract_method_other() {
        let body = br#"{"jsonrpc":"2.0","method":"tasks/get","params":{},"id":1}"#;
        assert_eq!(extract_method(body).await, Some("tasks/get".into()));
    }

    #[tokio::test]
    async fn extract_method_invalid_json() {
        assert_eq!(extract_method(b"not json").await, None);
    }
}
