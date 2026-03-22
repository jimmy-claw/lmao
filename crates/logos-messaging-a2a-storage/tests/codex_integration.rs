//! Integration tests against a real Codex (Logos Storage) node.
//!
//! These tests require a running Codex node at `CODEX_URL` (default: http://127.0.0.1:8090).
//! Start one with: `./scripts/start-codex.sh`
//!
//! Run with: `cargo test -p logos-messaging-a2a-storage --test codex_integration -- --ignored`

use logos_messaging_a2a_storage::{LogosStorageRest, StorageBackend};

fn codex_url() -> String {
    std::env::var("CODEX_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".to_string())
}

/// Check if Codex is reachable before running tests.
async fn require_codex() -> LogosStorageRest {
    let url = codex_url();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/storage/v1/debug/info", url))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => LogosStorageRest::new(&url),
        _ => panic!(
            "Codex node not reachable at {}. Start with: ./scripts/start-codex.sh",
            url
        ),
    }
}

#[tokio::test]
#[ignore] // requires running Codex node
async fn upload_download_roundtrip() {
    let backend = require_codex().await;

    let data = b"hello logos storage integration test".to_vec();
    let cid = backend.upload(data.clone()).await.expect("upload failed");

    assert!(!cid.is_empty(), "CID should not be empty");
    println!("Uploaded CID: {}", cid);

    let downloaded = backend.download(&cid).await.expect("download failed");
    assert_eq!(data, downloaded);
}

#[tokio::test]
#[ignore]
async fn upload_empty_data() {
    let backend = require_codex().await;

    let data = Vec::new();
    let result = backend.upload(data).await;
    // Codex may accept or reject empty uploads — document behavior
    match result {
        Ok(cid) => {
            println!("Empty upload returned CID: {}", cid);
            let downloaded = backend.download(&cid).await.expect("download empty failed");
            assert!(downloaded.is_empty());
        }
        Err(e) => {
            println!("Empty upload rejected (expected): {}", e);
        }
    }
}

#[tokio::test]
#[ignore]
async fn upload_large_payload() {
    let backend = require_codex().await;

    // 1 MB payload
    let data = vec![42u8; 1024 * 1024];
    let cid = backend
        .upload(data.clone())
        .await
        .expect("large upload failed");
    println!("Large upload CID: {}", cid);

    let downloaded = backend.download(&cid).await.expect("large download failed");
    assert_eq!(data.len(), downloaded.len());
    assert_eq!(data, downloaded);
}

#[tokio::test]
#[ignore]
async fn download_nonexistent_cid() {
    let backend = require_codex().await;

    let result = backend.download("zNonexistentCID12345").await;
    assert!(result.is_err(), "downloading nonexistent CID should fail");
    if let Err(e) = result {
        println!("Expected error for nonexistent CID: {}", e);
    }
}

#[tokio::test]
#[ignore]
async fn multiple_uploads_unique_cids() {
    let backend = require_codex().await;

    let cid1 = backend
        .upload(b"payload one".to_vec())
        .await
        .expect("upload 1");
    let cid2 = backend
        .upload(b"payload two".to_vec())
        .await
        .expect("upload 2");
    let cid3 = backend
        .upload(b"payload one".to_vec())
        .await
        .expect("upload 3 (dup)");

    assert_ne!(cid1, cid2, "different data should produce different CIDs");
    // Content-addressed: same data → same CID
    assert_eq!(cid1, cid3, "same data should produce same CID");
}
