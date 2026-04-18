//! [`StorageBackend`] implementation backed by `logos-storage-module`.
//!
//! `logos-storage-module` wraps `libstorage` and exposes a REST API with
//! lifecycle management (`init` → `start`) and file operations
//! (`upload_file`, `download_cid`).
//!
//! Use this backend when a `logos-storage-module` instance is running
//! (headless or GUI) and you want CID-based upload/download without embedding
//! a full Storage node in-process.

use crate::{StorageBackend, StorageError};

/// Configuration for connecting to a `logos-storage-module` instance.
#[derive(Debug, Clone)]
pub struct StorageModuleConfig {
    /// Base URL of the storage module REST API (e.g. `http://127.0.0.1:8090`).
    pub base_url: String,
    /// Optional throttle for upload bandwidth in bytes/sec. `None` means unlimited.
    pub upload_throttle: Option<u64>,
    /// Optional throttle for download bandwidth in bytes/sec. `None` means unlimited.
    pub download_throttle: Option<u64>,
}

impl StorageModuleConfig {
    /// Create a config pointing to the given base URL.
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            upload_throttle: None,
            download_throttle: None,
        }
    }

    /// Set the upload throttle in bytes/sec.
    pub fn with_upload_throttle(mut self, bytes_per_sec: u64) -> Self {
        self.upload_throttle = Some(bytes_per_sec);
        self
    }

    /// Set the download throttle in bytes/sec.
    pub fn with_download_throttle(mut self, bytes_per_sec: u64) -> Self {
        self.download_throttle = Some(bytes_per_sec);
        self
    }
}

impl Default for StorageModuleConfig {
    fn default() -> Self {
        Self::new("http://127.0.0.1:8090")
    }
}

/// Client for `logos-storage-module` REST API.
///
/// Lifecycle:
/// 1. Create with [`StorageModuleClient::new`] (sends `init` + `start` to the module).
/// 2. Use [`StorageBackend::upload`] / [`StorageBackend::download`] for CID operations.
///
/// The module must already be running and reachable at `config.base_url`.
pub struct StorageModuleClient {
    config: StorageModuleConfig,
    client: reqwest::Client,
}

impl std::fmt::Debug for StorageModuleClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageModuleClient")
            .field("config", &self.config)
            .finish()
    }
}

impl StorageModuleClient {
    /// Connect to an already-running `logos-storage-module` instance.
    ///
    /// Calls `POST /api/v1/init` and `POST /api/v1/start` to ensure the module
    /// is ready for uploads/downloads.
    pub async fn new(config: StorageModuleConfig) -> Result<Self, StorageError> {
        let client = reqwest::Client::new();
        let me = Self { config, client };

        me.init().await?;
        me.start().await?;

        Ok(me)
    }

    /// Create a client without calling init/start (assumes the module is already
    /// initialised and running).
    pub fn new_unchecked(config: StorageModuleConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// `POST /api/v1/init` — initialise the storage module with config.
    async fn init(&self) -> Result<(), StorageError> {
        let url = format!("{}/api/v1/init", self.config.base_url);

        let mut body = serde_json::Map::new();
        if let Some(up) = self.config.upload_throttle {
            body.insert(
                "uploadThrottle".to_string(),
                serde_json::Value::Number(up.into()),
            );
        }
        if let Some(dl) = self.config.download_throttle {
            body.insert(
                "downloadThrottle".to_string(),
                serde_json::Value::Number(dl.into()),
            );
        }

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| StorageError::Http(format!("init request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Api { status, body });
        }

        Ok(())
    }

    /// `POST /api/v1/start` — start the storage module.
    async fn start(&self) -> Result<(), StorageError> {
        let url = format!("{}/api/v1/start", self.config.base_url);

        let resp = self
            .client
            .post(&url)
            .send()
            .await
            .map_err(|e| StorageError::Http(format!("start request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Api { status, body });
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl StorageBackend for StorageModuleClient {
    /// Upload data via `POST /api/v1/upload`.
    ///
    /// Returns the CID assigned by the storage module.
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
        let url = format!("{}/api/v1/upload", self.config.base_url);

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            .map_err(|e| StorageError::Http(format!("upload request failed: {e}")))?;

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
            .map_err(|e| StorageError::Http(format!("reading upload response failed: {e}")))?
            .trim()
            .to_string();

        Ok(cid)
    }

    /// Download data by CID via `GET /api/v1/download/{cid}`.
    async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
        let url = format!("{}/api/v1/download/{}", self.config.base_url, cid);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| StorageError::Http(format!("download request failed: {e}")))?;

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
            .map_err(|e| StorageError::Http(format!("reading download response failed: {e}")))?;

        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_url() {
        let config = StorageModuleConfig::default();
        assert_eq!(config.base_url, "http://127.0.0.1:8090");
        assert!(config.upload_throttle.is_none());
        assert!(config.download_throttle.is_none());
    }

    #[test]
    fn config_trims_trailing_slash() {
        let config = StorageModuleConfig::new("http://localhost:8090/");
        assert_eq!(config.base_url, "http://localhost:8090");
    }

    #[test]
    fn config_with_throttles() {
        let config = StorageModuleConfig::new("http://localhost:8090")
            .with_upload_throttle(1_000_000)
            .with_download_throttle(500_000);
        assert_eq!(config.upload_throttle, Some(1_000_000));
        assert_eq!(config.download_throttle, Some(500_000));
    }

    #[test]
    fn new_unchecked_does_not_require_async() {
        let config = StorageModuleConfig::default();
        let client = StorageModuleClient::new_unchecked(config);
        assert_eq!(client.config.base_url, "http://127.0.0.1:8090");
    }

    #[tokio::test]
    async fn new_to_unreachable_host_returns_error() {
        let config = StorageModuleConfig::new("http://127.0.0.1:1");
        let result = StorageModuleClient::new(config).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Http(msg) => assert!(msg.contains("init request failed")),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn upload_to_unreachable_host_returns_error() {
        let config = StorageModuleConfig::new("http://127.0.0.1:1");
        let client = StorageModuleClient::new_unchecked(config);
        let result = client.upload(b"test".to_vec()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Http(msg) => assert!(msg.contains("upload request failed")),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn download_from_unreachable_host_returns_error() {
        let config = StorageModuleConfig::new("http://127.0.0.1:1");
        let client = StorageModuleClient::new_unchecked(config);
        let result = client.download("zQm123").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Http(msg) => assert!(msg.contains("download request failed")),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }
}
