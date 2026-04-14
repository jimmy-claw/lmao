//! Wiremock-based integration tests for [`StorageModuleClient`].
//!
//! These tests exercise the StorageModuleClient HTTP logic against a local
//! mock server simulating the logos-storage-module API.

#![cfg(feature = "storage-module")]

use logos_messaging_a2a_storage::{StorageBackend, StorageModuleClient, StorageModuleConfig};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn started_client(server: &MockServer) -> StorageModuleClient {
    Mock::given(method("GET"))
        .and(path("/api/v1/status"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status":"ok"}"#))
        .expect(1)
        .mount(server)
        .await;

    let mut client = StorageModuleClient::init(StorageModuleConfig::new(&server.uri()));
    client.start().await.expect("start failed");
    client
}

#[tokio::test]
async fn start_success() {
    let server = MockServer::start().await;
    let _client = started_client(&server).await;
}

#[tokio::test]
async fn start_fails_on_500() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/status"))
        .respond_with(ResponseTemplate::new(500).set_body_string("down"))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = StorageModuleClient::init(StorageModuleConfig::new(&server.uri()));
    let err = client.start().await.unwrap_err();
    match err {
        logos_messaging_a2a_storage::StorageError::Api { status, .. } => {
            assert_eq!(status, 500);
        }
        other => panic!("expected Api error, got: {:?}", other),
    }
}

#[tokio::test]
async fn upload_returns_cid() {
    let server = MockServer::start().await;
    let client = started_client(&server).await;

    Mock::given(method("POST"))
        .and(path("/api/v1/upload"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"cid":"zQmStorageModule123"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let cid = client.upload(b"hello storage module".to_vec()).await.unwrap();
    assert_eq!(cid, "zQmStorageModule123");
}

#[tokio::test]
async fn download_returns_bytes() {
    let server = MockServer::start().await;
    let client = started_client(&server).await;

    let payload = b"downloaded via storage module";
    Mock::given(method("GET"))
        .and(path("/api/v1/download/zQm456"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let data = client.download("zQm456").await.unwrap();
    assert_eq!(data, payload);
}

#[tokio::test]
async fn upload_download_roundtrip() {
    let server = MockServer::start().await;
    let client = started_client(&server).await;

    let original = b"roundtrip data".to_vec();
    let cid = "zQmRoundtrip";

    Mock::given(method("POST"))
        .and(path("/api/v1/upload"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(r#"{{"cid":"{}"}}"#, cid)),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/api/v1/download/{}", cid)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(original.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let returned_cid = client.upload(original.clone()).await.unwrap();
    assert_eq!(returned_cid, cid);

    let downloaded = client.download(&returned_cid).await.unwrap();
    assert_eq!(downloaded, original);
}

#[tokio::test]
async fn upload_server_error() {
    let server = MockServer::start().await;
    let client = started_client(&server).await;

    Mock::given(method("POST"))
        .and(path("/api/v1/upload"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client.upload(b"fail".to_vec()).await.unwrap_err();
    match err {
        logos_messaging_a2a_storage::StorageError::Api { status, body } => {
            assert_eq!(status, 500);
            assert_eq!(body, "internal error");
        }
        other => panic!("expected Api error, got: {:?}", other),
    }
}

#[tokio::test]
async fn download_404() {
    let server = MockServer::start().await;
    let client = started_client(&server).await;

    Mock::given(method("GET"))
        .and(path_regex(r"/api/v1/download/.+"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client.download("zNonexistent").await.unwrap_err();
    match err {
        logos_messaging_a2a_storage::StorageError::Api { status, body } => {
            assert_eq!(status, 404);
            assert_eq!(body, "not found");
        }
        other => panic!("expected Api error, got: {:?}", other),
    }
}

#[tokio::test]
async fn start_with_throttle() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/status"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status":"ok"}"#))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path("/api/v1/throttle"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let config = StorageModuleConfig::new(&server.uri())
        .with_upload_throttle(1_000_000)
        .with_download_throttle(500_000);

    let mut client = StorageModuleClient::init(config);
    client.start().await.expect("start with throttle failed");
}
