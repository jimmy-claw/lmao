//! [`StorageBackend`] implementation using `logos-storage-module` v0.3.2+.
//!
//! Talks to a running `logos-storage-module` instance via its REST API,
//! replacing direct Codex REST calls with the module's higher-level
//! upload/download endpoints that handle peer coordination, SDS bloom
//! filters, and throttle controls internally.

use crate::{StorageBackend, StorageError};

/// Default base URL for a local `logos-storage-module` instance.
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8090";

/// Storage backend using `logos-storage-module` REST API.
///
/// The module wraps Codex/libstorage with peer management, SDS bloom filters,
/// and throttle controls. Requires a running `logos-storage-module` instance
/// (headless mode recommended for server deployments).
///
/// # API surface used
///
/// | Method | Endpoint | Purpose |
/// |--------|----------|---------|
/// | `POST` | `/api/v1/upload` | Upload binary data, returns CID |
/// | `GET`  | `/api/v1/download/{cid}` | Download data by CID |
/// | `GET`  | `/api/v1/health` | Health check |
pub struct StorageModuleBackend {
    base_url: String,
    client: reqwest::Client,
}

impl StorageModuleBackend {
    /// Create a backend pointing to the given `logos-storage-module` URL.
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Create a backend using the default local URL (`http://127.0.0.1:8090`).
    pub fn default_local() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }

    /// Health check — returns `Ok(())` if the module is reachable and ready.
    pub async fn health(&self) -> Result<(), StorageError> {
        let url = format!("{}/api/v1/health", self.base_url);
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
        Ok(())
    }
}

#[async_trait::async_trait]
impl StorageBackend for StorageModuleBackend {
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
        let url = format!("{}/api/v1/upload", self.base_url);
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
        let url = format!("{}/api/v1/download/{}", self.base_url, cid);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_module_backend_url_construction() {
        let backend = StorageModuleBackend::new("http://localhost:8090/");
        assert_eq!(backend.base_url, "http://localhost:8090");
    }

    #[test]
    fn storage_module_backend_default_local() {
        let backend = StorageModuleBackend::default_local();
        assert_eq!(backend.base_url, "http://127.0.0.1:8090");
    }

    #[test]
    fn storage_module_backend_trims_trailing_slashes() {
        let backend = StorageModuleBackend::new("http://localhost:8090///");
        assert_eq!(backend.base_url, "http://localhost:8090");
    }

    #[test]
    fn storage_module_backend_no_trailing_slash() {
        let backend = StorageModuleBackend::new("http://localhost:8090");
        assert_eq!(backend.base_url, "http://localhost:8090");
    }

    #[test]
    fn storage_module_backend_custom_port() {
        let backend = StorageModuleBackend::new("http://storage.local:9000");
        assert_eq!(backend.base_url, "http://storage.local:9000");
    }

    #[tokio::test]
    async fn storage_module_upload_to_unreachable_host_returns_http_error() {
        let backend = StorageModuleBackend::new("http://127.0.0.1:1");
        let result = backend.upload(b"test".to_vec()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Http(msg) => assert!(!msg.is_empty()),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn storage_module_download_from_unreachable_host_returns_http_error() {
        let backend = StorageModuleBackend::new("http://127.0.0.1:1");
        let result = backend.download("zQm123").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Http(msg) => assert!(!msg.is_empty()),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn storage_module_health_unreachable_returns_error() {
        let backend = StorageModuleBackend::new("http://127.0.0.1:1");
        let result = backend.health().await;
        assert!(result.is_err());
    }
}
