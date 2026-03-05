//! [`StorageBackend`] implementation using `storage-bindings` (native FFI).
//!
//! Runs an embedded Storage node — no external process or REST API needed.

use crate::{StorageBackend, StorageError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use storage_bindings::{
    download_stream, upload_file, DownloadStreamOptions, LogLevel, StorageConfig, StorageNode,
    UploadOptions,
};

/// Native Storage backend using `storage-bindings` FFI crate.
///
/// Embeds a full Storage node in-process. The node is started on creation.
/// Call [`LibstorageBackend::shutdown`] to stop gracefully (consumes self).
pub struct LibstorageBackend {
    node: Arc<StorageNode>,
    /// Scratch directory for temp files (upload from bytes, download to bytes).
    scratch: PathBuf,
}

impl LibstorageBackend {
    /// Create and start a new embedded Storage node.
    ///
    /// * `data_dir` — persistent storage directory
    pub async fn new(data_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::with_config(data_dir, None, None).await
    }

    /// Create with explicit configuration options.
    pub async fn with_config(
        data_dir: impl AsRef<Path>,
        discovery_port: Option<u16>,
        storage_quota: Option<u64>,
    ) -> Result<Self, StorageError> {
        let data_dir = data_dir.as_ref();
        let scratch = data_dir.join("scratch");
        std::fs::create_dir_all(&scratch).map_err(|e| StorageError::Http(e.to_string()))?;

        let mut config = StorageConfig::new()
            .log_level(LogLevel::Warn)
            .data_dir(data_dir);

        if let Some(port) = discovery_port {
            config = config.discovery_port(port);
        }
        if let Some(quota) = storage_quota {
            config = config.storage_quota(quota);
        }

        let node = StorageNode::new(config)
            .await
            .map_err(|e| StorageError::Http(format!("failed to create storage node: {e}")))?;

        node.start()
            .await
            .map_err(|e| StorageError::Http(format!("failed to start storage node: {e}")))?;

        Ok(Self {
            node: Arc::new(node),
            scratch,
        })
    }

    /// Stop the embedded node gracefully (consumes self).
    pub async fn shutdown(self) -> Result<(), StorageError> {
        let node = Arc::try_unwrap(self.node).map_err(|_| {
            StorageError::Http("cannot shutdown: other references to node exist".into())
        })?;
        node.stop()
            .await
            .map_err(|e| StorageError::Http(format!("failed to stop storage node: {e}")))?;
        node.destroy()
            .await
            .map_err(|e| StorageError::Http(format!("failed to destroy storage node: {e}")))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl StorageBackend for LibstorageBackend {
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
        // Write data to a temp file, then upload via FFI
        let tmp = tempfile::NamedTempFile::new_in(&self.scratch)
            .map_err(|e| StorageError::Http(format!("temp file creation failed: {e}")))?;

        std::fs::write(tmp.path(), &data)
            .map_err(|e| StorageError::Http(format!("temp file write failed: {e}")))?;

        let upload_opts = UploadOptions::new().filepath(tmp.path());

        let result = upload_file(&self.node, upload_opts)
            .await
            .map_err(|e| StorageError::Http(format!("upload failed: {e}")))?;

        Ok(result.cid.to_string())
    }

    async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
        let download_path = self.scratch.join(format!("dl-{cid}"));

        let download_opts = DownloadStreamOptions::new(cid).filepath(&download_path);

        download_stream(&self.node, cid, download_opts)
            .await
            .map_err(|e| StorageError::Http(format!("download failed: {e}")))?;

        let data = std::fs::read(&download_path)
            .map_err(|e| StorageError::Http(format!("reading downloaded file failed: {e}")))?;

        // Clean up temp file
        let _ = std::fs::remove_file(&download_path);

        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorageBackend;
    use std::sync::atomic::{AtomicU16, Ordering};

    /// Allocate unique discovery ports so parallel tests don't collide.
    static NEXT_PORT: AtomicU16 = AtomicU16::new(19100);

    /// Helper: create a [`LibstorageBackend`] with a unique discovery port.
    async fn make_backend() -> (LibstorageBackend, tempfile::TempDir) {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let backend = LibstorageBackend::with_config(tmp.path(), Some(port), None)
            .await
            .expect("failed to create LibstorageBackend");
        (backend, tmp)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_libstorage_store_and_retrieve() {
        let (backend, _tmp) = make_backend().await;

        let data = b"hello libstorage roundtrip".to_vec();
        let cid = backend.upload(data.clone()).await.expect("upload failed");
        assert!(!cid.is_empty(), "CID should not be empty");

        let downloaded = backend.download(&cid).await.expect("download failed");
        assert_eq!(data, downloaded);

        backend.shutdown().await.expect("shutdown failed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_libstorage_store_empty() {
        let (backend, _tmp) = make_backend().await;

        let data = Vec::new();
        let result = backend.upload(data).await;
        // Storing empty bytes should either succeed or return an error — it must not panic.
        match result {
            Ok(cid) => assert!(!cid.is_empty()),
            Err(_) => { /* acceptable: backend may reject empty uploads */ }
        }

        backend.shutdown().await.expect("shutdown failed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_libstorage_retrieve_unknown_cid() {
        let (backend, _tmp) = make_backend().await;

        let result = backend.download("zNonexistentCid123456789").await;
        assert!(result.is_err(), "downloading an unknown CID should fail");

        backend.shutdown().await.expect("shutdown failed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_libstorage_multiple_stores() {
        let (backend, _tmp) = make_backend().await;

        let data_a = b"payload alpha".to_vec();
        let data_b = b"payload beta".to_vec();

        let cid_a = backend.upload(data_a.clone()).await.expect("upload A failed");
        let cid_b = backend.upload(data_b.clone()).await.expect("upload B failed");

        assert_ne!(cid_a, cid_b, "different payloads should yield different CIDs");

        let downloaded_a = backend.download(&cid_a).await.expect("download A failed");
        let downloaded_b = backend.download(&cid_b).await.expect("download B failed");

        assert_eq!(data_a, downloaded_a);
        assert_eq!(data_b, downloaded_b);

        backend.shutdown().await.expect("shutdown failed");
    }
}
