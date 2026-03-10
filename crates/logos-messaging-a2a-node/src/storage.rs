//! Configuration for offloading large payloads to Logos Storage.

use logos_messaging_a2a_storage::StorageBackend;
use std::sync::Arc;

/// Configuration for offloading large payloads to Logos Storage.
///
/// When a serialized message envelope exceeds `threshold_bytes`, the payload
/// is uploaded to storage and only the CID is sent over the Waku network.
/// The receiver automatically fetches the full payload by CID.
pub struct StorageOffloadConfig {
    /// Storage backend for uploading/downloading payloads.
    pub backend: Arc<dyn StorageBackend>,
    /// Payload size threshold in bytes. Payloads larger than this are offloaded.
    /// Default: 65 536 (64 KB).
    pub threshold_bytes: usize,
}

impl StorageOffloadConfig {
    /// Create a new config with the given backend and default threshold (64 KB).
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self {
            backend,
            threshold_bytes: 65_536,
        }
    }

    /// Create with a custom threshold.
    pub fn with_threshold(backend: Arc<dyn StorageBackend>, threshold_bytes: usize) -> Self {
        Self {
            backend,
            threshold_bytes,
        }
    }
}
