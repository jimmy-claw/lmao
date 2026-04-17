//! [`StorageBackend`] implementation matching the logos-storage-module API.
//!
//! Provides a `StorageModuleClient` that follows the logos-co/logos-storage-module
//! v0.3.2 lifecycle: `init(config) → start() → upload_file / download_cid`.
//!
//! Built on top of `storage-bindings` (the same `libstorage` FFI that
//! `logos-storage-module` uses internally), this backend can be swapped in as a
//! drop-in replacement for the lower-level [`LibstorageBackend`] with a
//! higher-level, module-oriented API.

use crate::{StorageBackend, StorageError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use storage_bindings::{
    download_stream, upload_file, DownloadStreamOptions, LogLevel, StorageConfig, StorageNode,
    UploadOptions,
};

/// Configuration for [`StorageModuleClient`].
///
/// Mirrors the configuration surface of `logos-storage-module`:
/// headless mode, throttle controls, discovery port, storage quota.
pub struct StorageModuleClientConfig {
    /// UDP port for peer discovery (`None` uses the default).
    pub discovery_port: Option<u16>,
    /// Maximum bytes the node may store (`None` for unlimited).
    pub storage_quota: Option<u64>,
    /// Log level for the embedded storage node.
    pub log_level: LogLevel,
}

impl Default for StorageModuleClientConfig {
    fn default() -> Self {
        Self {
            discovery_port: None,
            storage_quota: None,
            log_level: LogLevel::Warn,
        }
    }
}

impl StorageModuleClientConfig {
    /// Set the discovery port.
    pub fn discovery_port(mut self, port: u16) -> Self {
        self.discovery_port = Some(port);
        self
    }

    /// Set the storage quota in bytes.
    pub fn storage_quota(mut self, quota: u64) -> Self {
        self.storage_quota = Some(quota);
        self
    }
}

/// Storage backend using the logos-storage-module API pattern.
///
/// Manages a full storage node in-process following the
/// `init(config) → start() → upload_file / download_cid` lifecycle.
///
/// Call [`StorageModuleClient::shutdown`] to stop gracefully (consumes self).
///
/// # Example
///
/// ```no_run
/// use logos_messaging_a2a_storage::StorageModuleClient;
/// use logos_messaging_a2a_storage::StorageBackend;
///
/// # async fn example() -> Result<(), logos_messaging_a2a_storage::StorageError> {
/// let client = StorageModuleClient::new("/tmp/storage-data").await?;
///
/// let cid = client.upload(b"hello".to_vec()).await?;
/// let data = client.download(&cid).await?;
///
/// client.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct StorageModuleClient {
    node: Arc<StorageNode>,
    scratch: PathBuf,
}

impl StorageModuleClient {
    /// Initialise and start a storage module with default settings.
    ///
    /// Equivalent to `StorageModule::init(default_config).start()` in
    /// logos-storage-module.
    ///
    /// * `data_dir` — persistent storage directory
    pub async fn new(data_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::with_config(data_dir, StorageModuleClientConfig::default()).await
    }

    /// Initialise and start with explicit configuration.
    ///
    /// * `data_dir` — persistent storage directory
    /// * `config`   — module configuration
    pub async fn with_config(
        data_dir: impl AsRef<Path>,
        config: StorageModuleClientConfig,
    ) -> Result<Self, StorageError> {
        let data_dir = data_dir.as_ref();
        let scratch = data_dir.join("scratch");
        std::fs::create_dir_all(&scratch).map_err(|e| StorageError::Http(e.to_string()))?;

        let mut sc = StorageConfig::new()
            .log_level(config.log_level)
            .data_dir(data_dir);

        if let Some(port) = config.discovery_port {
            sc = sc.discovery_port(port);
        }
        if let Some(quota) = config.storage_quota {
            sc = sc.storage_quota(quota);
        }

        // init
        let node = StorageNode::new(sc)
            .await
            .map_err(|e| StorageError::Http(format!("failed to init storage module: {e}")))?;

        // start
        node.start()
            .await
            .map_err(|e| StorageError::Http(format!("failed to start storage module: {e}")))?;

        Ok(Self {
            node: Arc::new(node),
            scratch,
        })
    }

    /// Stop the storage module gracefully (consumes self).
    pub async fn shutdown(self) -> Result<(), StorageError> {
        let node = Arc::try_unwrap(self.node).map_err(|_| {
            StorageError::Http("cannot shutdown: other references to module exist".into())
        })?;
        node.stop()
            .await
            .map_err(|e| StorageError::Http(format!("failed to stop storage module: {e}")))?;
        node.destroy()
            .await
            .map_err(|e| StorageError::Http(format!("failed to destroy storage module: {e}")))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl StorageBackend for StorageModuleClient {
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
        let tmp = tempfile::NamedTempFile::new_in(&self.scratch)
            .map_err(|e| StorageError::Http(format!("temp file creation failed: {e}")))?;

        std::fs::write(tmp.path(), &data)
            .map_err(|e| StorageError::Http(format!("temp file write failed: {e}")))?;

        let opts = UploadOptions::new().filepath(tmp.path());

        let result = upload_file(&self.node, opts)
            .await
            .map_err(|e| StorageError::Http(format!("upload failed: {e}")))?;

        Ok(result.cid.to_string())
    }

    async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
        let download_path = self.scratch.join(format!("dl-{cid}"));

        let opts = DownloadStreamOptions::new(cid).filepath(&download_path);

        download_stream(&self.node, cid, opts)
            .await
            .map_err(|e| StorageError::Http(format!("download failed: {e}")))?;

        let data = std::fs::read(&download_path)
            .map_err(|e| StorageError::Http(format!("reading downloaded file failed: {e}")))?;

        let _ = std::fs::remove_file(&download_path);

        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorageBackend;
    use std::sync::atomic::{AtomicU16, Ordering};

    static NEXT_PORT: AtomicU16 = AtomicU16::new(19200);

    async fn make_client() -> (StorageModuleClient, tempfile::TempDir) {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        let tmp = tempfile::tempdir().expect("temp dir");
        let config = StorageModuleClientConfig::default().discovery_port(port);
        let client = StorageModuleClient::with_config(tmp.path(), config)
            .await
            .expect("failed to create StorageModuleClient");
        (client, tmp)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn roundtrip_upload_download() {
        let (client, _tmp) = make_client().await;

        let data = b"hello storage module client".to_vec();
        let cid = client.upload(data.clone()).await.expect("upload failed");
        assert!(!cid.is_empty());

        let downloaded = client.download(&cid).await.expect("download failed");
        assert_eq!(data, downloaded);

        client.shutdown().await.expect("shutdown failed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_unknown_cid_fails() {
        let (client, _tmp) = make_client().await;

        let result = client.download("zNonexistentCid123456789").await;
        assert!(result.is_err());

        client.shutdown().await.expect("shutdown failed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multiple_uploads_produce_different_cids() {
        let (client, _tmp) = make_client().await;

        let cid_a = client.upload(b"alpha".to_vec()).await.expect("upload A");
        let cid_b = client.upload(b"beta".to_vec()).await.expect("upload B");
        assert_ne!(cid_a, cid_b);

        client.shutdown().await.expect("shutdown failed");
    }

    #[test]
    fn default_config_values() {
        let config = StorageModuleClientConfig::default();
        assert!(config.discovery_port.is_none());
        assert!(config.storage_quota.is_none());
    }

    #[test]
    fn config_builder_methods() {
        let config = StorageModuleClientConfig::default()
            .discovery_port(9000)
            .storage_quota(1024 * 1024);
        assert_eq!(config.discovery_port, Some(9000));
        assert_eq!(config.storage_quota, Some(1024 * 1024));
    }
}
