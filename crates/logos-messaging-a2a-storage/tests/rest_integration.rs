//! Wiremock-based integration tests for [`LogosStorageRest`].
//!
//! These tests exercise the real HTTP request/response logic against a local
//! mock server — no real Codex node required.

use logos_messaging_a2a_storage::{maybe_offload, LogosStorageRest, StorageBackend, StorageError};
use wiremock::matchers::{body_bytes, header, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Upload tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upload_success_returns_cid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .respond_with(ResponseTemplate::new(200).set_body_string("zQmFakeCid123"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let cid = backend.upload(b"hello".to_vec()).await.unwrap();
    assert_eq!(cid, "zQmFakeCid123");
}

#[tokio::test]
async fn upload_server_error_returns_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let err = backend.upload(b"test".to_vec()).await.unwrap_err();
    match err {
        StorageError::Api { status, body } => {
            assert_eq!(status, 500);
            assert_eq!(body, "internal server error");
        }
        other => panic!("expected StorageError::Api, got: {:?}", other),
    }
}

#[tokio::test]
async fn upload_network_error_returns_http_error() {
    // Port 1 requires root — almost certainly not listening.
    let backend = LogosStorageRest::new("http://127.0.0.1:1");
    let err = backend.upload(b"data".to_vec()).await.unwrap_err();
    match err {
        StorageError::Http(msg) => {
            assert!(!msg.is_empty(), "error message should be non-empty");
        }
        other => panic!("expected StorageError::Http, got: {:?}", other),
    }
}

#[tokio::test]
async fn upload_sends_correct_content_type_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .and(header("Content-Type", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(200).set_body_string("zHeaderCid"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let cid = backend.upload(b"verify headers".to_vec()).await.unwrap();
    assert_eq!(cid, "zHeaderCid");
    // If the Content-Type header was wrong, wiremock would not match and the
    // `.expect(1)` assertion on drop would fail.
}

#[tokio::test]
async fn upload_trims_whitespace_from_cid_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .respond_with(ResponseTemplate::new(200).set_body_string("  zQmTrimMe  \n"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let cid = backend.upload(b"trim test".to_vec()).await.unwrap();
    assert_eq!(cid, "zQmTrimMe");
}

// ---------------------------------------------------------------------------
// Download tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn download_success_returns_bytes() {
    let server = MockServer::start().await;
    let payload = b"downloaded content";
    Mock::given(method("GET"))
        .and(path("/api/storage/v1/data/zQm123/network/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let data = backend.download("zQm123").await.unwrap();
    assert_eq!(data, payload);
}

#[tokio::test]
async fn download_404_returns_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/api/storage/v1/data/.+/network/stream"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let err = backend.download("zNonexistent").await.unwrap_err();
    match err {
        StorageError::Api { status, body } => {
            assert_eq!(status, 404);
            assert_eq!(body, "not found");
        }
        other => panic!("expected StorageError::Api, got: {:?}", other),
    }
}

#[tokio::test]
async fn download_network_error_returns_http_error() {
    let backend = LogosStorageRest::new("http://127.0.0.1:1");
    let err = backend.download("zAnyCid").await.unwrap_err();
    match err {
        StorageError::Http(msg) => {
            assert!(!msg.is_empty(), "error message should be non-empty");
        }
        other => panic!("expected StorageError::Http, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_upload_download_roundtrip() {
    let server = MockServer::start().await;
    let original = b"roundtrip payload".to_vec();
    let cid = "zQmRoundtrip42";

    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .and(body_bytes(original.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_string(cid))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/api/storage/v1/data/{}/network/stream", cid)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(original.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let returned_cid = backend.upload(original.clone()).await.unwrap();
    assert_eq!(returned_cid, cid);

    let downloaded = backend.download(&returned_cid).await.unwrap();
    assert_eq!(downloaded, original);
}

// ---------------------------------------------------------------------------
// URL construction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn url_construction_with_trailing_slash() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .respond_with(ResponseTemplate::new(200).set_body_string("zSlashCid"))
        .expect(1)
        .mount(&server)
        .await;

    // Append trailing slashes — LogosStorageRest should trim them.
    let backend = LogosStorageRest::new(&format!("{}/", server.uri()));
    let cid = backend.upload(b"slash test".to_vec()).await.unwrap();
    assert_eq!(cid, "zSlashCid");
}

#[tokio::test]
async fn url_construction_with_multiple_trailing_slashes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/storage/v1/data/zCid/network/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&format!("{}///", server.uri()));
    let data = backend.download("zCid").await.unwrap();
    assert_eq!(data, b"ok");
}

// ---------------------------------------------------------------------------
// maybe_offload with real REST backend
// ---------------------------------------------------------------------------

#[tokio::test]
async fn maybe_offload_above_threshold_uses_rest_backend() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .respond_with(ResponseTemplate::new(200).set_body_string("zOffloaded"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let data = vec![0xffu8; 200];
    let result = maybe_offload(&backend, &data, 100).await.unwrap();
    assert_eq!(result, Some("zOffloaded".to_string()));
}

#[tokio::test]
async fn maybe_offload_below_threshold_does_not_call_server() {
    let server = MockServer::start().await;
    // Mount no mocks — any request will be unmatched and wiremock will return 404.
    // We rely on the fact that maybe_offload should NOT make a request at all.

    let backend = LogosStorageRest::new(&server.uri());
    let data = vec![0u8; 50];
    let result = maybe_offload(&backend, &data, 100).await.unwrap();
    assert!(result.is_none());

    // Verify zero requests were made.
    let received = server.received_requests().await.unwrap();
    assert!(
        received.is_empty(),
        "expected no HTTP requests below threshold, got {}",
        received.len()
    );
}

#[tokio::test]
async fn maybe_offload_at_exact_threshold_does_not_upload() {
    let server = MockServer::start().await;

    let backend = LogosStorageRest::new(&server.uri());
    let data = vec![0u8; 100];
    let result = maybe_offload(&backend, &data, 100).await.unwrap();
    assert!(result.is_none());

    let received = server.received_requests().await.unwrap();
    assert!(received.is_empty());
}

// ---------------------------------------------------------------------------
// Large and empty payloads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn large_payload_upload_download() {
    let server = MockServer::start().await;
    let large = vec![0xab_u8; 512 * 1024]; // 512 KB

    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .respond_with(ResponseTemplate::new(200).set_body_string("zLargeCid"))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/storage/v1/data/zLargeCid/network/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(large.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());

    let cid = backend.upload(large.clone()).await.unwrap();
    assert_eq!(cid, "zLargeCid");

    let downloaded = backend.download(&cid).await.unwrap();
    assert_eq!(downloaded.len(), large.len());
    assert_eq!(downloaded, large);
}

#[tokio::test]
async fn empty_payload_upload_download() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .and(body_bytes(Vec::<u8>::new()))
        .respond_with(ResponseTemplate::new(200).set_body_string("zEmptyCid"))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/storage/v1/data/zEmptyCid/network/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(Vec::<u8>::new()))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());

    let cid = backend.upload(Vec::new()).await.unwrap();
    assert_eq!(cid, "zEmptyCid");

    let downloaded = backend.download(&cid).await.unwrap();
    assert!(downloaded.is_empty());
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upload_409_conflict_returns_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .respond_with(ResponseTemplate::new(409).set_body_string("conflict"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let err = backend.upload(b"dup".to_vec()).await.unwrap_err();
    match err {
        StorageError::Api { status, body } => {
            assert_eq!(status, 409);
            assert_eq!(body, "conflict");
        }
        other => panic!("expected StorageError::Api, got: {:?}", other),
    }
}

#[tokio::test]
async fn download_503_returns_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/api/storage/v1/data/.+/network/stream"))
        .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let err = backend.download("zSomeCid").await.unwrap_err();
    match err {
        StorageError::Api { status, body } => {
            assert_eq!(status, 503);
            assert_eq!(body, "service unavailable");
        }
        other => panic!("expected StorageError::Api, got: {:?}", other),
    }
}

#[tokio::test]
async fn upload_sends_exact_body_bytes() {
    let server = MockServer::start().await;
    let payload = vec![0x00, 0x01, 0x02, 0xff, 0xfe, 0xfd];

    Mock::given(method("POST"))
        .and(path("/api/storage/v1/data"))
        .and(body_bytes(payload.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_string("zBinaryCid"))
        .expect(1)
        .mount(&server)
        .await;

    let backend = LogosStorageRest::new(&server.uri());
    let cid = backend.upload(payload).await.unwrap();
    assert_eq!(cid, "zBinaryCid");
}
