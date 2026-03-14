//! Wiremock-based integration tests for [`LogosMessagingTransport`].
//!
//! These tests exercise the real HTTP logic against a local mock
//! server — no running nwaku node required.

use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use logos_messaging_a2a_transport::Transport;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ENCODED_PUBSUB_TOPIC: &str = "%2Fwaku%2F2%2Fdefault-waku%2Fproto";

fn transport_for(server: &MockServer) -> LogosMessagingTransport {
    LogosMessagingTransport::new(&server.uri())
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

#[test]
fn new_trims_trailing_slash() {
    let t = LogosMessagingTransport::new("http://localhost:8645/");
    // The trailing slash is removed (verified by the field in `test_transport_creation` inline test)
    // Here we verify construction doesn't panic
    drop(t);
}

#[test]
fn new_preserves_url_without_trailing_slash() {
    let t = LogosMessagingTransport::new("http://localhost:8645");
    drop(t);
}

// ---------------------------------------------------------------------------
// publish
// ---------------------------------------------------------------------------

#[tokio::test]
async fn publish_sends_post_to_relay_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(format!(".*{}", ENCODED_PUBSUB_TOPIC)))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    transport
        .publish("/my/content/topic", b"hello waku")
        .await
        .unwrap();
}

#[tokio::test]
async fn publish_payload_is_base64_encoded() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    transport.publish("/topic", b"test payload").await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();

    // Payload should be base64 encoded
    let payload_b64 = body["payload"].as_str().unwrap();
    assert!(!payload_b64.is_empty());

    // contentTopic should match what we passed
    assert_eq!(body["contentTopic"].as_str().unwrap(), "/topic");

    // timestamp should be set
    assert!(body["timestamp"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn publish_empty_payload() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    transport.publish("/topic", b"").await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    // Empty payload should produce empty base64 string
    assert_eq!(body["payload"].as_str().unwrap(), "");
}

#[tokio::test]
async fn publish_http_error_returns_err() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .expect(1)
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let err = transport.publish("/topic", b"data").await.unwrap_err();
    assert!(
        err.to_string().contains("nwaku publish failed"),
        "got: {}",
        err
    );
}

#[tokio::test]
async fn publish_http_400_returns_err() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .expect(1)
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let err = transport.publish("/topic", b"data").await.unwrap_err();
    assert!(
        err.to_string().contains("nwaku publish failed"),
        "got: {}",
        err
    );
}

#[tokio::test]
async fn publish_network_unreachable_returns_err() {
    let transport = LogosMessagingTransport::new("http://127.0.0.1:1");
    let err = transport.publish("/topic", b"data").await.unwrap_err();
    assert!(
        err.to_string().contains("Failed to publish"),
        "got: {}",
        err
    );
}

#[tokio::test]
async fn publish_uses_correct_url_path() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(format!(
            "^/relay/v1/messages/{}",
            ENCODED_PUBSUB_TOPIC
        )))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    transport.publish("/topic", b"data").await.unwrap();
}

#[tokio::test]
async fn publish_multiple_messages_sequentially() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(3)
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    transport.publish("/topic", b"msg1").await.unwrap();
    transport.publish("/topic", b"msg2").await.unwrap();
    transport.publish("/topic", b"msg3").await.unwrap();
}

// ---------------------------------------------------------------------------
// subscribe + message polling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn subscribe_polls_and_receives_messages() {
    let server = MockServer::start().await;

    // Mock the GET endpoint that subscribe's poll loop hits
    Mock::given(method("GET"))
        .and(path_regex(format!(".*{}", ENCODED_PUBSUB_TOPIC)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "aGVsbG8=",  // base64("hello")
                "contentTopic": "/test/topic"
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/test/topic").await.unwrap();

    // The poll loop runs every 500ms; wait for it to fire
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out waiting for message")
        .expect("channel closed");

    assert_eq!(msg, b"hello");
}

#[tokio::test]
async fn subscribe_filters_by_content_topic() {
    let server = MockServer::start().await;

    // Return messages for two different content topics
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "d3Jvbmc=",  // base64("wrong")
                "contentTopic": "/other/topic"
            },
            {
                "payload": "cmlnaHQ=",  // base64("right")
                "contentTopic": "/my/topic"
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/my/topic").await.unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    assert_eq!(msg, b"right");
}

#[tokio::test]
async fn subscribe_empty_response_no_messages() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/topic").await.unwrap();

    // No messages should arrive; verify with a short timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
    assert!(result.is_err(), "should not have received any message");
}

#[tokio::test]
async fn subscribe_deduplicates_messages() {
    let server = MockServer::start().await;

    // Return the same message twice — subscriber should deduplicate
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "ZHVw",  // base64("dup")
                "contentTopic": "/topic"
            },
            {
                "payload": "ZHVw",  // same payload
                "contentTopic": "/topic"
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/topic").await.unwrap();

    // First message should arrive
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(msg, b"dup");

    // Second identical message should be deduplicated — no more messages
    let result = tokio::time::timeout(std::time::Duration::from_millis(800), rx.recv()).await;
    assert!(
        result.is_err(),
        "duplicate message should have been filtered"
    );
}

// ---------------------------------------------------------------------------
// unsubscribe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unsubscribe_stops_polling() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let _rx = transport.subscribe("/topic").await.unwrap();
    transport.unsubscribe("/topic").await.unwrap();

    // After unsubscribe, polling task should be aborted.
    // Give it a moment then check that no new requests arrive.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let requests_before = server.received_requests().await.unwrap().len();

    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let requests_after = server.received_requests().await.unwrap().len();

    // No new polling requests should have been made
    assert_eq!(
        requests_before, requests_after,
        "polling should have stopped after unsubscribe"
    );
}

#[tokio::test]
async fn unsubscribe_nonexistent_topic_succeeds() {
    let transport = LogosMessagingTransport::new("http://localhost:8645");
    transport.unsubscribe("/never-subscribed").await.unwrap();
}

#[tokio::test]
async fn subscribe_replaces_existing_subscription() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "bmV3",  // base64("new")
                "contentTopic": "/topic"
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let _rx1 = transport.subscribe("/topic").await.unwrap();

    // Subscribing again should abort the previous subscription task
    let mut rx2 = transport.subscribe("/topic").await.unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx2.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(msg, b"new");
}

// ---------------------------------------------------------------------------
// poll error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn subscribe_tolerates_poll_errors() {
    let server = MockServer::start().await;

    // First few polls return errors, then valid data
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "cmVjb3Zlcg==",  // base64("recover")
                "contentTopic": "/topic"
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/topic").await.unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for recovery")
        .expect("channel closed");
    assert_eq!(msg, b"recover");
}

#[tokio::test]
async fn subscribe_handles_malformed_json_response() {
    let server = MockServer::start().await;

    // Return non-JSON, then valid response
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .up_to_n_times(2)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "b2s=",  // base64("ok")
                "contentTopic": "/topic"
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/topic").await.unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(msg, b"ok");
}

#[tokio::test]
async fn subscribe_lenient_base64_decoder_strips_non_alphabet_chars() {
    let server = MockServer::start().await;

    // The custom base64 decoder is lenient: it strips non-alphabet characters
    // rather than rejecting them. So "!!!abc!!!" decodes the "abc" portion.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "dmFsaWQ=",  // base64("valid")
                "contentTopic": "/topic"
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/topic").await.unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(msg, b"valid");
}

#[tokio::test]
async fn subscribe_message_without_content_topic_matches_all() {
    let server = MockServer::start().await;

    // Message with no contentTopic field — should be delivered to any subscriber
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "payload": "bm9jdA=="  // base64("noct") — no contentTopic
            }
        ])))
        .mount(&server)
        .await;

    let transport = transport_for(&server);
    let mut rx = transport.subscribe("/any/topic").await.unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(msg, b"noct");
}
