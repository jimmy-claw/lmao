//! Wiremock-based integration tests for [`LezExecutionBackend`].
//!
//! These tests exercise the JSON-RPC bridge logic against a local mock
//! server — no real LEZ sequencer or Logos Core module required.

use logos_messaging_a2a_core::AgentCard;
use logos_messaging_a2a_execution::{AgentId, ExecutionBackend, LezExecutionBackend};
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_card() -> AgentCard {
    AgentCard {
        name: "lez-agent".into(),
        description: "A LEZ test agent".into(),
        version: "0.1.0".into(),
        capabilities: vec!["payments".into()],
        public_key: "0xdeadbeef".into(),
        intro_bundle: None,
    }
}

const WALLET: &str = "0xagent_wallet_address";

fn backend_for(server: &MockServer, spending_limit: u64) -> LezExecutionBackend {
    LezExecutionBackend::new(&server.uri(), WALLET, spending_limit)
}

fn rpc_success(result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "result": result, "id": 1 })
}

fn rpc_error(code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "error": { "code": code, "message": message },
        "id": 1
    })
}

fn dummy_tx_hash() -> String {
    format!("0x{}", "aa".repeat(32))
}

// ---------------------------------------------------------------------------
// register_agent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_agent_sends_correct_rpc() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "lez_registerAgent" }),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(dummy_tx_hash()))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let tx = backend.register_agent(&test_card()).await.unwrap();
    assert_eq!(tx.to_string().len(), 64);
}

#[tokio::test]
async fn register_agent_includes_card_fields() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(dummy_tx_hash()))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    backend.register_agent(&test_card()).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let params = &body["params"];
    assert_eq!(params["name"], "lez-agent");
    assert_eq!(params["publicKey"], "0xdeadbeef");
    assert_eq!(params["wallet"], WALLET);
}

#[tokio::test]
async fn register_agent_rpc_error_propagates() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_error(-32000, "registration failed")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let err = backend.register_agent(&test_card()).await.unwrap_err();
    assert!(err.to_string().contains("LEZ module error"));
}

// ---------------------------------------------------------------------------
// pay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pay_sends_correct_rpc() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "lez_sendTransaction" }),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(dummy_tx_hash()))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let tx = backend.pay(&AgentId("0xrecipient".into()), 500).await.unwrap();
    assert_eq!(tx.to_string().len(), 64);
}

#[tokio::test]
async fn pay_includes_amount_and_addresses() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(dummy_tx_hash()))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    backend.pay(&AgentId("0xrecipient".into()), 750).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let params = &body["params"];
    assert_eq!(params["from"], WALLET);
    assert_eq!(params["to"], "0xrecipient");
    assert_eq!(params["amount"], 750);
}

#[tokio::test]
async fn pay_tracks_cumulative_spend() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(dummy_tx_hash()))),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    backend.pay(&AgentId("0xa".into()), 100).await.unwrap();
    backend.pay(&AgentId("0xb".into()), 200).await.unwrap();
    assert_eq!(backend.total_spent(), 300);
}

#[tokio::test]
async fn pay_rejects_over_spending_limit() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(dummy_tx_hash()))),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server, 500);
    backend.pay(&AgentId("0xa".into()), 400).await.unwrap();

    // This should fail — 400 + 200 > 500
    let err = backend.pay(&AgentId("0xb".into()), 200).await.unwrap_err();
    assert!(err.to_string().contains("Spending limit exceeded"));
    // Total should still be 400 (failed payment not recorded)
    assert_eq!(backend.total_spent(), 400);
}

#[tokio::test]
async fn pay_allows_exact_limit() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(dummy_tx_hash()))),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server, 1000);
    backend.pay(&AgentId("0xa".into()), 1000).await.unwrap();
    assert_eq!(backend.total_spent(), 1000);
    assert_eq!(backend.remaining_allowance(), 0);
}

#[tokio::test]
async fn pay_rpc_error_does_not_record_spend() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_error(-32000, "insufficient funds")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let _ = backend.pay(&AgentId("0xa".into()), 100).await;
    assert_eq!(backend.total_spent(), 0);
}

// ---------------------------------------------------------------------------
// balance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn balance_parses_hex_string() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "lez_getBalance" }),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x3e8"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let bal = backend.balance(&AgentId("0xagent".into())).await.unwrap();
    assert_eq!(bal, 1000);
}

#[tokio::test]
async fn balance_parses_integer() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(42))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let bal = backend.balance(&AgentId("0xagent".into())).await.unwrap();
    assert_eq!(bal, 42);
}

#[tokio::test]
async fn balance_zero() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x0"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let bal = backend.balance(&AgentId("0xagent".into())).await.unwrap();
    assert_eq!(bal, 0);
}

#[tokio::test]
async fn balance_sends_agent_address() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x0"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    backend.balance(&AgentId("0xmyagent".into())).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["params"]["address"], "0xmyagent");
}

// ---------------------------------------------------------------------------
// verify_transfer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn verify_transfer_success() {
    let server = MockServer::start().await;

    let receipt = serde_json::json!({
        "status": "confirmed",
        "from": "0xsender",
        "to": "0xrecipient",
        "amount": 500,
        "blockNumber": 42
    });

    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "lez_getTransactionReceipt" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let details = backend.verify_transfer("0xdeadbeef").await.unwrap();
    assert_eq!(details.from, "0xsender");
    assert_eq!(details.to, "0xrecipient");
    assert_eq!(details.amount, 500);
    assert_eq!(details.block_number, 42);
}

#[tokio::test]
async fn verify_transfer_evm_status() {
    let server = MockServer::start().await;

    let receipt = serde_json::json!({
        "status": "0x1",
        "from": "0xa",
        "to": "0xb",
        "amount": 1,
        "blockNumber": "0xa"
    });

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let details = backend.verify_transfer("0x1234").await.unwrap();
    assert_eq!(details.block_number, 10); // 0xa
}

#[tokio::test]
async fn verify_transfer_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(null))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let err = backend.verify_transfer("0xnotfound").await.unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn verify_transfer_failed_status() {
    let server = MockServer::start().await;

    let receipt = serde_json::json!({
        "status": "failed",
        "from": "0xa",
        "to": "0xb",
        "amount": 0,
        "blockNumber": 1
    });

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let err = backend.verify_transfer("0xfailed").await.unwrap_err();
    assert!(err.to_string().contains("failed"));
}

#[tokio::test]
async fn verify_transfer_normalizes_hash() {
    let server = MockServer::start().await;

    let receipt = serde_json::json!({
        "status": "confirmed",
        "from": "0xa",
        "to": "0xb",
        "amount": 1,
        "blockNumber": 1
    });

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    // No 0x prefix
    backend.verify_transfer("deadbeef").await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let sent_hash = body["params"]["hash"].as_str().unwrap();
    assert!(sent_hash.starts_with("0x"));
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bridge_unreachable_returns_error() {
    let backend = LezExecutionBackend::new("http://127.0.0.1:1", "0xw", 0);
    let err = backend.balance(&AgentId("0xa".into())).await.unwrap_err();
    assert!(err.to_string().contains("LEZ bridge unreachable"));
}

#[tokio::test]
async fn malformed_response_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let err = backend.balance(&AgentId("0xa".into())).await.unwrap_err();
    assert!(err.to_string().contains("LEZ bridge bad response"));
}

#[tokio::test]
async fn missing_result_field_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server, 0);
    let err = backend.balance(&AgentId("0xa".into())).await.unwrap_err();
    assert!(err.to_string().contains("Missing result"));
}
