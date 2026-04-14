//! [`StorageBackend`] implementation using `logos-storage-module` v0.3.2+.
//!
//! Replaces direct Codex REST calls with the higher-level
//! `logos-storage-module` API that handles SDS bloom-filter coordination,
//! peer discovery, and throttle controls automatically.
//!
//! # Lifecycle
//!
//! ```text
//! StorageModuleClient::init(config) → .start() → .upload_file() / .download_cid()
//! ```
//!
//! The module manages its own Codex node connection and storage coordination.

use crate::{StorageBackend, StorageError};

/// Configuration for [`StorageModuleClient`].
#[derive(Debug, Clone)]
pub struct StorageModuleConfig {
    /// Base URL of the logos-storage-module API (e.g. `http://127.0.0.1:8090`).
    pub base_url: String,
    /// Enable headless mode (no interactive prompts). Default: `true`.
    pub headless: bool,
    /// Upload throttle in bytes/sec. `None` for unlimited.
    pub upload_throttle: Option<u64>,
    /// Download throttle in bytes/sec. `None` for unlimited.
    pub download_throttle: Option<u64>,
}

impl StorageModuleConfig {
    /// Create a config targeting the given base URL with defaults.
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            headless: true,
            upload_throttle: None,
            download_throttle: None,
        }
    }

    /// Default local configuration (`http://127.0.0.1:8090`, headless).
    pub fn default_local() -> Self {
        Self::new("http://127.0.0.1:8090")
    }

    /// Set upload throttle.
    pub fn with_upload_throttle(mut self, bytes_per_sec: u64) -> Self {
        self.upload_throttle = Some(bytes_per_sec);
        self
    }

    /// Set download throttle.
    pub fn with_download_throttle(mut self, bytes_per_sec: u64) -> Self {
        self.download_throttle = Some(bytes_per_sec);
        self
    }
}

/// Storage backend using `logos-storage-module` API.
///
/// Provides CID-based upload/download through the storage module's REST API,
/// which handles SDS bloom-filter coordination and peer management internally.
///
/// # API Endpoints
///
/// - Upload:   `POST {base_url}/api/v1/upload`   (multipart file upload, returns JSON `{"cid": "..."}`)
/// - Download: `GET  {base_url}/api/v1/download/{cid}` (returns raw bytes)
/// - Status:   `GET  {base_url}/api/v1/status`    (module health check)
#[cfg(feature = "storage-module")]
pub struct StorageModuleClient {
    config: StorageModuleConfig,
    client: reqwest::Client,
    started: bool,
}

#[cfg(feature = "storage-module")]
impl StorageModuleClient {
    /// Initialize a new storage module client. Call [`start`](Self::start) before use.
    pub fn init(config: StorageModuleConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            started: false,
        }
    }

    /// Start the client by verifying the storage module is reachable.
    ///
    /// Checks the module's `/api/v1/status` endpoint and optionally applies
    /// throttle settings.
    pub async fn start(&mut self) -> Result<(), StorageError> {
        // Verify module is reachable.
        let status_url = format!("{}/api/v1/status", self.config.base_url);
        let resp = self
            .client
            .get(&status_url)
            .send()
            .await
            .map_err(|e| StorageError::Http(format!("storage module unreachable: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Api {
                status: status.as_u16(),
                body: format!("storage module status check failed: {body}"),
            });
        }

        // Apply throttle settings if configured.
        if self.config.upload_throttle.is_some() || self.config.download_throttle.is_some() {
            self.apply_throttle().await?;
        }

        self.started = true;
        Ok(())
    }

    /// Upload a file and return its CID.
    pub async fn upload_file(&self, data: Vec<u8>) -> Result<String, StorageError> {
        if !self.started {
            return Err(StorageError::Http(
                "storage module not started; call start() first".into(),
            ));
        }

        let url = format!("{}/api/v1/upload", self.config.base_url);
        let part = reqwest::multipart::Part::bytes(data).file_name("blob");
        let form = reqwest::multipart::Form::new().part("file", part);

        let resp = self
            .client
            .post(&url)
            .multipart(form)
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

        // Response: {"cid": "zQm..."}
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| StorageError::Http(format!("invalid upload response: {e}")))?;

        body["cid"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                StorageError::Http(format!("upload response missing 'cid' field: {body}"))
            })
    }

    /// Download data by CID.
    pub async fn download_cid(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
        if !self.started {
            return Err(StorageError::Http(
                "storage module not started; call start() first".into(),
            ));
        }

        let url = format!("{}/api/v1/download/{}", self.config.base_url, cid);
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

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| StorageError::Http(e.to_string()))
    }

    /// Apply throttle configuration to the running module.
    async fn apply_throttle(&self) -> Result<(), StorageError> {
        let url = format!("{}/api/v1/throttle", self.config.base_url);
        let mut body = serde_json::Map::new();

        if let Some(up) = self.config.upload_throttle {
            body.insert(
                "upload_bytes_per_sec".into(),
                serde_json::Value::Number(up.into()),
            );
        }
        if let Some(down) = self.config.download_throttle {
            body.insert(
                "download_bytes_per_sec".into(),
                serde_json::Value::Number(down.into()),
            );
        }

        let resp = self
            .client
            .put(&url)
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .map_err(|e| StorageError::Http(format!("throttle config failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Api {
                status: status.as_u16(),
                body: format!("throttle config rejected: {body}"),
            });
        }

        Ok(())
    }
}

/// Implement [`StorageBackend`] so `StorageModuleClient` can be used as a
/// drop-in replacement for `LogosStorageRest`.
#[cfg(feature = "storage-module")]
#[async_trait::async_trait]
impl StorageBackend for StorageModuleClient {
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
        self.upload_file(data).await
    }

    async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
        self.download_cid(cid).await
    }
}

#[cfg(all(test, feature = "storage-module"))]
mod tests {
    use super::*;

    #[test]
    fn config_default_local() {
        let cfg = StorageModuleConfig::default_local();
        assert_eq!(cfg.base_url, "http://127.0.0.1:8090");
        assert!(cfg.headless);
        assert!(cfg.upload_throttle.is_none());
        assert!(cfg.download_throttle.is_none());
    }

    #[test]
    fn config_trims_trailing_slash() {
        let cfg = StorageModuleConfig::new("http://localhost:8090/");
        assert_eq!(cfg.base_url, "http://localhost:8090");
    }

    #[test]
    fn config_with_throttle() {
        let cfg = StorageModuleConfig::default_local()
            .with_upload_throttle(1_000_000)
            .with_download_throttle(500_000);
        assert_eq!(cfg.upload_throttle, Some(1_000_000));
        assert_eq!(cfg.download_throttle, Some(500_000));
    }

    #[test]
    fn init_creates_unstarted_client() {
        let client = StorageModuleClient::init(StorageModuleConfig::default_local());
        assert!(!client.started);
    }

    #[tokio::test]
    async fn upload_before_start_returns_error() {
        let client = StorageModuleClient::init(StorageModuleConfig::default_local());
        let err = client.upload(b"test".to_vec()).await.unwrap_err();
        match err {
            StorageError::Http(msg) => assert!(msg.contains("not started")),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn download_before_start_returns_error() {
        let client = StorageModuleClient::init(StorageModuleConfig::default_local());
        let err = client.download("zQm123").await.unwrap_err();
        match err {
            StorageError::Http(msg) => assert!(msg.contains("not started")),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn start_with_unreachable_module_returns_error() {
        let mut client =
            StorageModuleClient::init(StorageModuleConfig::new("http://127.0.0.1:1"));
        let err = client.start().await.unwrap_err();
        match err {
            StorageError::Http(msg) => assert!(msg.contains("unreachable")),
            other => panic!("expected Http error, got: {:?}", other),
        }
    }
}
