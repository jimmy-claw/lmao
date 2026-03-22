//! Wiremock-based integration tests for [`StatusNetworkBackend`].
//!
//! These tests exercise the real HTTP/JSON-RPC logic against a local mock
//! server — no real blockchain node required.

use logos_messaging_a2a_core::AgentCard;
use logos_messaging_a2a_execution::{AgentId, ExecutionBackend, StatusNetworkBackend};
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_card() -> AgentCard {
    AgentCard {
        name: "test-agent".into(),
        description: "A test agent".into(),
        version: "0.1.0".into(),
        capabilities: vec!["echo".into(), "ping".into()],
        public_key: "0xdeadbeef".into(),
        intro_bundle: None,
    }
}

const TOKEN_CONTRACT: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REGISTRY_CONTRACT: &str = "0x438bB48f4E3Dd338e98e3EEf7b5dDc90a1490b8C";

fn backend_for(server: &MockServer) -> StatusNetworkBackend {
    StatusNetworkBackend::new(&server.uri(), REGISTRY_CONTRACT, TOKEN_CONTRACT)
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

/// ERC-20 Transfer event topic.
const TRANSFER_TOPIC: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// Build a successful transaction receipt with a Transfer log.
fn receipt_with_transfer(from_addr: &str, to_addr: &str, amount_hex: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "0x1",
        "blockNumber": "0xa",
        "logs": [{
            "address": TOKEN_CONTRACT,
            "topics": [
                TRANSFER_TOPIC,
                format!("0x000000000000000000000000{}", from_addr),
                format!("0x000000000000000000000000{}", to_addr),
            ],
            "data": format!("0x{:0>64}", amount_hex),
        }]
    })
}

// ---------------------------------------------------------------------------
// register_agent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_agent_sends_eth_call_with_abi_encoded_data() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "eth_call" }),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let tx = backend.register_agent(&test_card()).await.unwrap();
    // TxHash is 32 bytes displayed as 64 hex chars
    assert_eq!(tx.to_string().len(), 64);
}

#[tokio::test]
async fn register_agent_targets_registry_contract_with_latest_block() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    backend.register_agent(&test_card()).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let params = body["params"].as_array().unwrap();

    // First param: call object with "to" = registry and "data" starting with 0x
    let to = params[0]["to"].as_str().unwrap();
    assert_eq!(to, REGISTRY_CONTRACT);

    let data = params[0]["data"].as_str().unwrap();
    assert!(data.starts_with("0x"), "calldata must start with 0x");
    assert!(data.len() > 10, "calldata must contain ABI-encoded params");

    // Second param: "latest"
    assert_eq!(params[1].as_str().unwrap(), "latest");
}

#[tokio::test]
async fn register_agent_deterministic_for_same_card() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x"))),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let card = test_card();
    let tx1 = backend.register_agent(&card).await.unwrap();
    let tx2 = backend.register_agent(&card).await.unwrap();
    assert_eq!(tx1, tx2);
}

#[tokio::test]
async fn register_agent_rpc_error_propagates() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_error(-32000, "execution reverted")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.register_agent(&test_card()).await.unwrap_err();
    assert!(err.to_string().contains("RPC error"), "got: {}", err);
}

// ---------------------------------------------------------------------------
// balance (get_balance)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn balance_returns_decoded_hex_value() {
    let server = MockServer::start().await;

    // 0x64 = 100
    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "eth_call" }),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!(
                "0x0000000000000000000000000000000000000000000000000000000000000064"
            ))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let bal = backend
        .balance(&AgentId(
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        ))
        .await
        .unwrap();
    assert_eq!(bal, 100);
}

#[tokio::test]
async fn balance_sends_balance_of_selector() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x0"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    backend.balance(&AgentId("0xabcdef".into())).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let params = body["params"].as_array().unwrap();
    let data = params[0]["data"].as_str().unwrap();

    // balanceOf(address) selector = 0x70a08231
    assert!(
        data.starts_with("0x70a08231"),
        "expected balanceOf selector, got: {}",
        data
    );

    // "to" should be the token contract
    assert_eq!(params[0]["to"].as_str().unwrap(), TOKEN_CONTRACT);
    assert_eq!(params[1].as_str().unwrap(), "latest");
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

    let backend = backend_for(&server);
    let bal = backend.balance(&AgentId("0xabc".into())).await.unwrap();
    assert_eq!(bal, 0);
}

#[tokio::test]
async fn balance_max_u64() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(rpc_success(serde_json::json!("0xffffffffffffffff"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let bal = backend.balance(&AgentId("0xabc".into())).await.unwrap();
    assert_eq!(bal, u64::MAX);
}

#[tokio::test]
async fn balance_rpc_error_propagates() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_error(-32602, "invalid params")))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.balance(&AgentId("0xabc".into())).await.unwrap_err();
    assert!(err.to_string().contains("RPC error"));
}

// ---------------------------------------------------------------------------
// pay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pay_calls_eth_chain_id() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "eth_chainId" }),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x1"))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let tx = backend
        .pay(&AgentId("0xrecipient".into()), 500)
        .await
        .unwrap();
    assert_eq!(tx.to_string().len(), 64);
}

#[tokio::test]
async fn pay_deterministic_for_same_inputs() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x1"))),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let tx1 = backend
        .pay(&AgentId("0xrecipient".into()), 500)
        .await
        .unwrap();
    let tx2 = backend
        .pay(&AgentId("0xrecipient".into()), 500)
        .await
        .unwrap();
    assert_eq!(tx1, tx2);
}

#[tokio::test]
async fn pay_different_amounts_produce_different_hashes() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_success(serde_json::json!("0x1"))),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let tx1 = backend
        .pay(&AgentId("0xrecipient".into()), 100)
        .await
        .unwrap();
    let tx2 = backend
        .pay(&AgentId("0xrecipient".into()), 200)
        .await
        .unwrap();
    assert_ne!(tx1, tx2);
}

#[tokio::test]
async fn pay_rpc_error_propagates() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_error(-32000, "chain unavailable")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend
        .pay(&AgentId("0xrecipient".into()), 100)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("RPC error"));
}

// ---------------------------------------------------------------------------
// verify_transfer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn verify_transfer_success() {
    let server = MockServer::start().await;

    let receipt = receipt_with_transfer(
        "1111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222",
        "3e8", // 1000
    );

    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "method": "eth_getTransactionReceipt" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let details = backend
        .verify_transfer("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
        .await
        .unwrap();

    assert_eq!(details.from, "0x1111111111111111111111111111111111111111");
    assert_eq!(details.to, "0x2222222222222222222222222222222222222222");
    assert_eq!(details.amount, 1000);
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

    let backend = backend_for(&server);
    let err = backend.verify_transfer("0xnonexistent").await.unwrap_err();
    assert!(err.to_string().contains("not found"), "got: {}", err);
}

#[tokio::test]
async fn verify_transfer_failed_status() {
    let server = MockServer::start().await;

    let receipt = serde_json::json!({
        "status": "0x0",
        "blockNumber": "0x1",
        "logs": []
    });

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.verify_transfer("0xfailedtx").await.unwrap_err();
    assert!(err.to_string().contains("failed"), "got: {}", err);
}

#[tokio::test]
async fn verify_transfer_no_transfer_event() {
    let server = MockServer::start().await;

    let receipt = serde_json::json!({
        "status": "0x1",
        "blockNumber": "0x5",
        "logs": [{
            "address": TOKEN_CONTRACT,
            "topics": [
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ],
            "data": "0x00"
        }]
    });

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.verify_transfer("0xnotransfer").await.unwrap_err();
    assert!(err.to_string().contains("Transfer"), "got: {}", err);
}

#[tokio::test]
async fn verify_transfer_empty_logs() {
    let server = MockServer::start().await;

    let receipt = serde_json::json!({
        "status": "0x1",
        "blockNumber": "0x1",
        "logs": []
    });

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.verify_transfer("0xemptylogs").await.unwrap_err();
    assert!(err.to_string().contains("Transfer"));
}

#[tokio::test]
async fn verify_transfer_normalizes_hash_without_0x_prefix() {
    let server = MockServer::start().await;

    let receipt = receipt_with_transfer(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "1",
    );

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_success(receipt)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    // No 0x prefix — should still work
    let details = backend
        .verify_transfer("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
        .await
        .unwrap();
    assert_eq!(details.amount, 1);

    // Verify the backend normalised the hash to include 0x
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let sent_hash = body["params"][0].as_str().unwrap();
    assert!(
        sent_hash.starts_with("0x"),
        "hash should be normalised to include 0x prefix"
    );
}

#[tokio::test]
async fn verify_transfer_rpc_error_propagates() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_error(-32000, "node syncing")))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.verify_transfer("0xabc").await.unwrap_err();
    assert!(err.to_string().contains("RPC error"));
}

// ---------------------------------------------------------------------------
// Error handling: RPC errors, missing results, malformed responses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rpc_error_method_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rpc_error(-32601, "method not found")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.balance(&AgentId("0xabc".into())).await.unwrap_err();
    assert!(err.to_string().contains("RPC error"));
    assert!(err.to_string().contains("method not found"));
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

    let backend = backend_for(&server);
    let err = backend.balance(&AgentId("0xabc".into())).await.unwrap_err();
    assert!(err.to_string().contains("Missing result"), "got: {}", err);
}

#[tokio::test]
async fn malformed_json_response_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("this is not json"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.balance(&AgentId("0xabc".into())).await.unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[tokio::test]
async fn http_500_with_non_json_body_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.balance(&AgentId("0xabc".into())).await.unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[tokio::test]
async fn network_unreachable_returns_error() {
    let backend = StatusNetworkBackend::new("http://127.0.0.1:1", "0xREG", "0xTOK");
    let err = backend.balance(&AgentId("0xabc".into())).await.unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[tokio::test]
async fn rpc_error_message_is_preserved() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(rpc_error(-32000, "gas required exceeds allowance")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.register_agent(&test_card()).await.unwrap_err();
    assert!(err.to_string().contains("gas required exceeds allowance"));
}

#[tokio::test]
async fn empty_json_object_response_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let backend = backend_for(&server);
    let err = backend.balance(&AgentId("0xabc".into())).await.unwrap_err();
    assert!(err.to_string().contains("Missing result"), "got: {}", err);
}
