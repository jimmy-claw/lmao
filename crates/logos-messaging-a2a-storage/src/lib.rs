//! Storage backend abstraction for Logos Messaging A2A.
//!
//! Provides a [`StorageBackend`] trait for uploading/downloading binary payloads
//! and two concrete implementations:
//!
//! | Backend | Feature flag | When to use |
//! |---------|-------------|-------------|
//! | [`LogosStorageRest`] | `rest` (default) | Standalone processes talking to a Codex REST API |
//! | [`LogosCoreStorageBackend`] | `logos-core` | Inside a Logos Core host process (desktop client) |
//!
//! # Example (REST)
//!
//! ```no_run
//! use logos_messaging_a2a_storage::{LogosStorageRest, StorageBackend};
//!
//! # async fn example() -> Result<(), logos_messaging_a2a_storage::StorageError> {
//! let backend = LogosStorageRest::new("http://127.0.0.1:8080");
//!
//! // Upload
//! let data = b"hello world".to_vec();
//! let cid = backend.upload(data.clone()).await?;
//!
//! // Download
//! let downloaded = backend.download(&cid).await?;
//! assert_eq!(data, downloaded);
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "logos-core")]
mod logos_core;
#[cfg(feature = "logos-core")]
mod logos_core_backend;
#[cfg(feature = "logos-core")]
pub use logos_core_backend::LogosCoreStorageBackend;

#[cfg(feature = "libstorage")]
mod libstorage_backend;
#[cfg(feature = "libstorage")]
pub use libstorage_backend::LibstorageBackend;

use std::fmt;

/// Errors returned by storage operations.
#[derive(Debug)]
pub enum StorageError {
    /// HTTP or network-level failure.
    Http(String),
    /// Non-success status code from the storage API.
    Api { status: u16, body: String },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Http(msg) => write!(f, "storage HTTP error: {}", msg),
            StorageError::Api { status, body } => {
                write!(f, "storage API error ({}): {}", status, body)
            }
        }
    }
}

impl std::error::Error for StorageError {}

/// Trait for uploading and downloading binary payloads to/from content-addressed storage.
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    /// Upload binary data, returning the content identifier (CID).
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError>;

    /// Download binary data by CID.
    async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError>;
}

/// Logos Storage (Codex) REST API backend.
///
/// - Upload: `POST {base_url}/api/storage/v1/data` with `Content-Type: application/octet-stream`
/// - Download: `GET {base_url}/api/storage/v1/data/{cid}/network/stream`
#[cfg(feature = "rest")]
pub struct LogosStorageRest {
    base_url: String,
    client: reqwest::Client,
}

#[cfg(feature = "rest")]
impl LogosStorageRest {
    /// Create a new backend targeting the given Codex base URL.
    ///
    /// Default Codex URL: `http://127.0.0.1:8080`
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Create a backend using the default local Codex URL (`http://127.0.0.1:8080`).
    pub fn default_local() -> Self {
        Self::new("http://127.0.0.1:8080")
    }
}

#[cfg(feature = "rest")]
#[async_trait::async_trait]
impl StorageBackend for LogosStorageRest {
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
        let url = format!("{}/api/storage/v1/data", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            .map_err(|e| StorageError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let cid = resp
            .text()
            .await
            .map_err(|e| StorageError::Http(e.to_string()))?
            .trim()
            .to_string();
        Ok(cid)
    }

    async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
        let url = format!(
            "{}/api/storage/v1/data/{}/network/stream",
            self.base_url, cid
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| StorageError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| StorageError::Http(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

/// Default offload threshold: 100 KB.
pub const DEFAULT_OFFLOAD_THRESHOLD: usize = 100 * 1024;

/// If `data` exceeds `threshold` bytes, upload to storage and return the CID.
/// Otherwise return `None`.
pub async fn maybe_offload(
    backend: &dyn StorageBackend,
    data: &[u8],
    threshold: usize,
) -> Result<Option<String>, StorageError> {
    if data.len() > threshold {
        let cid = backend.upload(data.to_vec()).await?;
        Ok(Some(cid))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory mock storage backend for testing.
    struct MockStorage {
        store: Mutex<HashMap<String, Vec<u8>>>,
        next_id: Mutex<u64>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                store: Mutex::new(HashMap::new()),
                next_id: Mutex::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl StorageBackend for MockStorage {
        async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
            let mut id = self.next_id.lock().unwrap();
            let cid = format!("zMock{}", *id);
            *id += 1;
            self.store.lock().unwrap().insert(cid.clone(), data);
            Ok(cid)
        }

        async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
            self.store
                .lock()
                .unwrap()
                .get(cid)
                .cloned()
                .ok_or_else(|| StorageError::Api {
                    status: 404,
                    body: format!("CID not found: {}", cid),
                })
        }
    }

    #[tokio::test]
    async fn upload_download_roundtrip() {
        let backend = MockStorage::new();
        let data = b"hello logos storage".to_vec();

        let cid = backend.upload(data.clone()).await.unwrap();
        assert!(cid.starts_with("zMock"));

        let downloaded = backend.download(&cid).await.unwrap();
        assert_eq!(data, downloaded);
    }

    #[tokio::test]
    async fn download_missing_cid_returns_error() {
        let backend = MockStorage::new();
        let result = backend.download("zNonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Api { status, .. } => assert_eq!(status, 404),
            other => panic!("expected Api error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn maybe_offload_below_threshold_returns_none() {
        let backend = MockStorage::new();
        let small_data = vec![0u8; 100];
        let result = maybe_offload(&backend, &small_data, 1024).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn maybe_offload_above_threshold_uploads() {
        let backend = MockStorage::new();
        let big_data = vec![42u8; 2048];
        let result = maybe_offload(&backend, &big_data, 1024).await.unwrap();
        assert!(result.is_some());

        let cid = result.unwrap();
        let downloaded = backend.download(&cid).await.unwrap();
        assert_eq!(big_data, downloaded);
    }

    #[tokio::test]
    async fn maybe_offload_exact_threshold_returns_none() {
        let backend = MockStorage::new();
        let exact_data = vec![0u8; 1024];
        let result = maybe_offload(&backend, &exact_data, 1024).await.unwrap();
        assert!(
            result.is_none(),
            "data at exactly the threshold should not be offloaded"
        );
    }

    #[test]
    fn storage_error_display() {
        let http_err = StorageError::Http("connection refused".to_string());
        assert!(http_err.to_string().contains("connection refused"));

        let api_err = StorageError::Api {
            status: 500,
            body: "internal".to_string(),
        };
        assert!(api_err.to_string().contains("500"));
    }

    #[cfg(feature = "rest")]
    #[test]
    fn logos_storage_rest_url_construction() {
        let backend = LogosStorageRest::new("http://localhost:8080/");
        assert_eq!(backend.base_url, "http://localhost:8080");

        let backend2 = LogosStorageRest::default_local();
        assert_eq!(backend2.base_url, "http://127.0.0.1:8080");
    }
}
