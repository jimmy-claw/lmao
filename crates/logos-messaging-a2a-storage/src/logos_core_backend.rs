//! `LogosCoreStorageBackend` — native Logos Core storage via `logos_core_call_plugin_method_async`.
//!
//! This backend calls the `storage_module` plugin through the Logos Core C API:
//!
//! ```text
//! Rust → logos_core_call_plugin_method_async("storage_module", …)
//!      → Qt Remote Objects → storage_module plugin → libstorage → Codex node
//! ```
//!
//! Use this backend when running inside a Logos Core host process (desktop client,
//! embedded node). For standalone / REST access, use
//! [`LogosStorageRest`](crate::LogosStorageRest) instead.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

use base64::Engine as _;
use tokio::sync::{mpsc, oneshot};

use crate::logos_core;
use crate::StorageError;

const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;
const PLUGIN: &str = "storage_module";

/// Storage backend that calls `storage_module` via `logos_core_call_plugin_method_async`.
///
/// # When to use
///
/// Use `LogosCoreStorageBackend` when running inside a Logos Core host process
/// (desktop client, embedded node). The calls travel through Qt Remote Objects
/// to the `storage_module` plugin and from there into `libstorage` / Codex.
///
/// If you are **not** inside a Logos Core process, use
/// [`LogosStorageRest`](crate::LogosStorageRest) to talk directly to the Codex REST API.
pub struct LogosCoreStorageBackend {
    chunk_size: usize,
}

impl LogosCoreStorageBackend {
    /// Create a new backend with the default 64 KB chunk size.
    pub fn new() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    /// Create a new backend with a custom chunk size.
    pub fn with_chunk_size(chunk_size: usize) -> Self {
        Self { chunk_size }
    }
}

impl Default for LogosCoreStorageBackend {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `params_json` string for `logos_core_call_plugin_method_async`.
///
/// Each entry is `{"name": …, "value": …, "type": …}`.
fn params_json(params: &[(&str, &str, &str)]) -> String {
    let arr: Vec<String> = params
        .iter()
        .map(|(name, value, ty)| {
            format!(
                r#"{{"name":"{}","value":"{}","type":"{}"}}"#,
                name, value, ty,
            )
        })
        .collect();
    format!("[{}]", arr.join(","))
}

/// Call a `storage_module` plugin method and await the one-shot callback.
async fn call_method(method: &str, params: &str) -> Result<String, StorageError> {
    let (tx, rx) = oneshot::channel::<Result<String, StorageError>>();

    let plugin = CString::new(PLUGIN).unwrap();
    let method = CString::new(method).unwrap();
    let params = CString::new(params).unwrap();

    let user_data = Box::into_raw(Box::new(tx)) as *mut c_void;

    unsafe {
        logos_core::logos_core_call_plugin_method_async(
            plugin.as_ptr(),
            method.as_ptr(),
            params.as_ptr(),
            method_callback,
            user_data,
        );
    }

    rx.await
        .map_err(|_| StorageError::Http("callback channel closed".into()))?
}

/// C-ABI callback for one-shot method calls.
///
/// Reconstructs the `oneshot::Sender` from `user_data` and sends the result.
extern "C" fn method_callback(result: c_int, message: *const c_char, user_data: *mut c_void) {
    // SAFETY: `user_data` was created via `Box::into_raw` in `call_method`.
    let tx =
        unsafe { Box::from_raw(user_data as *mut oneshot::Sender<Result<String, StorageError>>) };

    let msg = if message.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };

    let value = if result == 0 {
        Ok(msg)
    } else {
        Err(StorageError::Http(format!(
            "plugin error ({}): {}",
            result, msg
        )))
    };

    let _ = tx.send(value);
}

/// Register an event listener on `storage_module` and return the receiver end.
///
/// The `mpsc::UnboundedSender` is leaked into `user_data` so it lives as long
/// as the event listener. When the receiver is dropped, subsequent sends from
/// the callback become no-ops.
fn register_event_listener(
    event_name: &str,
) -> mpsc::UnboundedReceiver<Result<String, StorageError>> {
    let (tx, rx) = mpsc::unbounded_channel();

    let plugin = CString::new(PLUGIN).unwrap();
    let event = CString::new(event_name).unwrap();

    let user_data = Box::into_raw(Box::new(tx)) as *mut c_void;

    unsafe {
        logos_core::logos_core_register_event_listener(
            plugin.as_ptr(),
            event.as_ptr(),
            event_callback,
            user_data,
        );
    }

    rx
}

/// C-ABI callback for event listeners.
///
/// Does **not** reclaim ownership of `user_data` — the sender lives for the
/// lifetime of the listener registration.
extern "C" fn event_callback(result: c_int, message: *const c_char, user_data: *mut c_void) {
    // SAFETY: `user_data` points to a leaked `Box<UnboundedSender<…>>`.
    let tx =
        unsafe { &*(user_data as *const mpsc::UnboundedSender<Result<String, StorageError>>) };

    let msg = if message.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };

    let value = if result == 0 {
        Ok(msg)
    } else {
        Err(StorageError::Http(format!(
            "event error ({}): {}",
            result, msg
        )))
    };

    let _ = tx.send(value);
}

// ---------------------------------------------------------------------------
// StorageBackend impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl crate::StorageBackend for LogosCoreStorageBackend {
    async fn upload(&self, data: Vec<u8>) -> Result<String, StorageError> {
        let engine = base64::engine::general_purpose::STANDARD;

        // 1. Listen for the upload-done event before starting.
        let mut done_rx = register_event_listener("storageUploadDone");

        // 2. uploadInit → session ID.
        let init_params = params_json(&[
            ("filename", "blob", "string"),
            ("chunkSize", &self.chunk_size.to_string(), "int"),
        ]);
        let session_id = call_method("uploadInit", &init_params).await?;

        // 3. Upload chunks (base64-encoded for QByteArray transport).
        for chunk in data.chunks(self.chunk_size) {
            let encoded = engine.encode(chunk);
            let chunk_params = params_json(&[
                ("sessionId", &session_id, "string"),
                ("chunk", &encoded, "string"),
            ]);
            call_method("uploadChunk", &chunk_params).await?;
        }

        // 4. Finalize the upload session.
        let fin_params = params_json(&[("sessionId", &session_id, "string")]);
        call_method("uploadFinalize", &fin_params).await?;

        // 5. Wait for "storageUploadDone" event carrying the CID.
        let cid = done_rx
            .recv()
            .await
            .ok_or_else(|| StorageError::Http("storageUploadDone channel closed".into()))??;

        Ok(cid)
    }

    async fn download(&self, cid: &str) -> Result<Vec<u8>, StorageError> {
        let engine = base64::engine::general_purpose::STANDARD;

        // 1. Register event listeners before triggering the download.
        let mut chunk_rx = register_event_listener("storageDownloadChunk");
        let mut done_rx = register_event_listener("storageDownloadDone");

        // 2. Start the download.
        let dl_params = params_json(&[
            ("cid", cid, "string"),
            ("local", "false", "bool"),
        ]);
        call_method("downloadChunks", &dl_params).await?;

        // 3. Collect chunks until the "done" event fires.
        let mut assembled = Vec::new();

        loop {
            tokio::select! {
                Some(chunk_result) = chunk_rx.recv() => {
                    let chunk_b64 = chunk_result?;
                    let bytes = engine.decode(&chunk_b64).map_err(|e| {
                        StorageError::Http(format!("base64 decode error: {e}"))
                    })?;
                    assembled.extend_from_slice(&bytes);
                }
                Some(done_result) = done_rx.recv() => {
                    done_result?;
                    break;
                }
            }
        }

        Ok(assembled)
    }
}
