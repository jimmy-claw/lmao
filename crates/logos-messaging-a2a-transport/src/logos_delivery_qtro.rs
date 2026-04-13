//! QtRO-based delivery transport — inter-module calls via Logos Core's QtRemoteObjects layer.
//!
//! Instead of linking `liblogos_core.so` at compile time (like the `logos-core` feature),
//! this transport receives function pointers at runtime from the C++ module host.
//! The C++ side calls `logosAPI->getClient("delivery_module")` to obtain a QtRO replica,
//! then injects publish/subscribe/unsubscribe callbacks via [`set_qtro_callbacks`].
//!
//! This is the same pattern used by `logos-kv-module` and `lez-multisig-module`.

use crate::{Result, Transport, TransportError};
use async_trait::async_trait;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Callback function pointer types (set by C++ host at runtime)
// ---------------------------------------------------------------------------

/// `int publish(const char* topic, const char* payload_b64, void* user_data)`
/// Returns 0 on success, non-zero on error.
pub type PublishFn = extern "C" fn(topic: *const c_char, payload_b64: *const c_char, user_data: *mut c_void) -> c_int;

/// `int subscribe(const char* topic, void* user_data)`
/// Returns 0 on success, non-zero on error.
pub type SubscribeFn = extern "C" fn(topic: *const c_char, user_data: *mut c_void) -> c_int;

/// `int unsubscribe(const char* topic, void* user_data)`
/// Returns 0 on success, non-zero on error.
pub type UnsubscribeFn = extern "C" fn(topic: *const c_char, user_data: *mut c_void) -> c_int;

/// Callback table injected by the C++ module host.
#[repr(C)]
pub struct QtROCallbacks {
    pub publish: PublishFn,
    pub subscribe: SubscribeFn,
    pub unsubscribe: UnsubscribeFn,
    /// Opaque pointer passed back to every callback (e.g. pointer to QtRO replica wrapper).
    pub user_data: *mut c_void,
}

// SAFETY: The C++ side guarantees that user_data points to a thread-safe object
// (QRemoteObjectDynamicReplica calls are serialised by Qt's event loop).
unsafe impl Send for QtROCallbacks {}
unsafe impl Sync for QtROCallbacks {}

/// Global callback table — set once by `set_qtro_callbacks`, read by every transport instance.
static CALLBACKS: OnceLock<QtROCallbacks> = OnceLock::new();

/// Register the QtRO callback table. Called once from C++ during module initialisation.
///
/// # Safety
/// `cbs` must point to a valid `QtROCallbacks` whose function pointers and `user_data`
/// remain valid for the lifetime of the process.
pub unsafe fn set_qtro_callbacks(cbs: QtROCallbacks) -> bool {
    CALLBACKS.set(cbs).is_ok()
}

fn callbacks() -> Result<&'static QtROCallbacks> {
    CALLBACKS
        .get()
        .ok_or_else(|| TransportError::Transport("QtRO callbacks not initialised — call set_qtro_callbacks first".into()))
}

// ---------------------------------------------------------------------------
// Inbound message dispatch (C++ → Rust)
// ---------------------------------------------------------------------------

/// Per-topic sender map for dispatching inbound messages from the C++ event handler.
static TOPIC_SENDERS: OnceLock<Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>>> =
    OnceLock::new();

fn topic_senders() -> &'static Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>> {
    TOPIC_SENDERS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Called from C++ when the delivery_module emits a message on a subscribed topic.
///
/// # Safety
/// `topic` and `payload_b64` must be valid, null-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn lmao_qtro_on_message(topic: *const c_char, payload_b64: *const c_char) {
    let topic_str = match unsafe { CStr::from_ptr(topic) }.to_str() {
        Ok(s) => s,
        Err(_) => return,
    };
    let payload_str = match unsafe { CStr::from_ptr(payload_b64) }.to_str() {
        Ok(s) => s,
        Err(_) => return,
    };

    let payload = match base64_decode(payload_str) {
        Some(p) => p,
        None => return,
    };

    let guard = topic_senders().lock().unwrap();
    if let Some(tx) = guard.get(topic_str) {
        let _ = tx.send(payload);
    }
}

// ---------------------------------------------------------------------------
// Transport implementation
// ---------------------------------------------------------------------------

/// Logos Delivery transport via QtRO inter-module calls.
///
/// Requires [`set_qtro_callbacks`] to have been called before use.
/// The C++ module host obtains a `delivery_module` QtRO replica via
/// `logosAPI->getClient("delivery_module")` and wires the callbacks.
pub struct QtRODeliveryTransport {
    _private: (),
}

impl QtRODeliveryTransport {
    /// Create a new QtRO delivery transport.
    ///
    /// Fails if [`set_qtro_callbacks`] has not been called.
    pub fn new() -> Result<Self> {
        callbacks()?; // verify callbacks are registered
        Ok(Self { _private: () })
    }
}

#[async_trait]
impl Transport for QtRODeliveryTransport {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let cbs = callbacks()?;
        let topic_c = CString::new(topic)
            .map_err(|_| TransportError::Transport("topic contains null byte".into()))?;
        let payload_b64 = base64_encode(payload);
        let payload_c = CString::new(payload_b64)
            .map_err(|_| TransportError::Transport("base64 payload contains null byte".into()))?;

        let rc = (cbs.publish)(topic_c.as_ptr(), payload_c.as_ptr(), cbs.user_data);
        if rc != 0 {
            return Err(TransportError::Transport(format!(
                "QtRO publish failed (rc={})",
                rc
            )));
        }
        Ok(())
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let cbs = callbacks()?;
        let topic_c = CString::new(topic)
            .map_err(|_| TransportError::Transport("topic contains null byte".into()))?;

        let rc = (cbs.subscribe)(topic_c.as_ptr(), cbs.user_data);
        if rc != 0 {
            return Err(TransportError::Transport(format!(
                "QtRO subscribe failed (rc={})",
                rc
            )));
        }

        // Register a channel for inbound messages on this topic.
        let (tx, rx_unbounded) = mpsc::unbounded_channel();
        topic_senders()
            .lock()
            .unwrap()
            .insert(topic.to_string(), tx);

        // Bridge unbounded → bounded channel (backpressure).
        let (btx, brx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut rx = rx_unbounded;
            while let Some(msg) = rx.recv().await {
                if btx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        Ok(brx)
    }

    async fn unsubscribe(&self, topic: &str) -> Result<()> {
        // Remove local sender first.
        topic_senders().lock().unwrap().remove(topic);

        let cbs = callbacks()?;
        let topic_c = CString::new(topic)
            .map_err(|_| TransportError::Transport("topic contains null byte".into()))?;

        let rc = (cbs.unsubscribe)(topic_c.as_ptr(), cbs.user_data);
        if rc != 0 {
            return Err(TransportError::Transport(format!(
                "QtRO unsubscribe failed (rc={})",
                rc
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Minimal base64 helpers (no external dep needed)
// ---------------------------------------------------------------------------

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    }
    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        let c = val(chunk[2])?;
        let d = val(chunk[3])?;
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        out.push((triple >> 16) as u8);
        if chunk[2] != b'=' {
            out.push((triple >> 8) as u8);
        }
        if chunk[3] != b'=' {
            out.push(triple as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        let cases: &[&[u8]] = &[b"", b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"];
        for case in cases {
            let encoded = base64_encode(case);
            let decoded = base64_decode(&encoded).unwrap();
            assert_eq!(&decoded, case, "roundtrip failed for {:?}", case);
        }
    }

    #[test]
    fn base64_encode_known() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
        assert_eq!(base64_encode(b"Hello!"), "SGVsbG8h");
    }

    #[test]
    fn base64_decode_invalid() {
        assert!(base64_decode("abc").is_none()); // not multiple of 4
    }

    #[test]
    fn transport_new_without_callbacks_fails() {
        // CALLBACKS is a OnceLock — if not set, new() should fail.
        // Note: this test may pass or fail depending on test ordering since OnceLock is global.
        // We test the error path only if callbacks haven't been set yet.
        if CALLBACKS.get().is_none() {
            let result = QtRODeliveryTransport::new();
            assert!(result.is_err());
        }
    }

    #[test]
    fn topic_senders_dispatch() {
        let senders = topic_senders();
        let (tx, mut rx) = mpsc::unbounded_channel();
        senders.lock().unwrap().insert("test/topic".into(), tx);

        let topic = CString::new("test/topic").unwrap();
        let payload = base64_encode(b"hello");
        let payload_c = CString::new(payload).unwrap();

        unsafe {
            lmao_qtro_on_message(topic.as_ptr(), payload_c.as_ptr());
        }

        let received = rx.try_recv().unwrap();
        assert_eq!(received, b"hello");

        // Clean up
        senders.lock().unwrap().remove("test/topic");
    }

    #[test]
    fn on_message_unknown_topic_ignored() {
        let topic = CString::new("no/such/topic").unwrap();
        let payload = CString::new(base64_encode(b"data")).unwrap();

        // Should not panic
        unsafe {
            lmao_qtro_on_message(topic.as_ptr(), payload.as_ptr());
        }
    }

    #[test]
    fn on_message_invalid_base64_ignored() {
        let senders = topic_senders();
        let (tx, mut rx) = mpsc::unbounded_channel();
        senders.lock().unwrap().insert("b64test".into(), tx);

        let topic = CString::new("b64test").unwrap();
        let payload = CString::new("not-valid-base64!").unwrap();

        unsafe {
            lmao_qtro_on_message(topic.as_ptr(), payload.as_ptr());
        }

        // Should not have dispatched anything
        assert!(rx.try_recv().is_err());

        senders.lock().unwrap().remove("b64test");
    }

    #[test]
    fn callbacks_struct_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<QtROCallbacks>();
    }
}
