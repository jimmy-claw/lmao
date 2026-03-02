//! FFI bindings for Logos Core plugin IPC.
//!
//! Wraps `logos_core_call_plugin_method_async` and `logos_core_register_event_listener`
//! from the Logos Core C API into safe async Rust helpers.

use std::ffi::{c_char, c_int, c_void, CString};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};

/// Callback signature matching `AsyncCallback` in the Logos Core C API:
/// `typedef void (*AsyncCallback)(int result, const char* message, void* user_data);`
type AsyncCallback = extern "C" fn(result: c_int, message: *const c_char, user_data: *mut c_void);

extern "C" {
    fn logos_core_call_plugin_method_async(
        plugin_name: *const c_char,
        method_name: *const c_char,
        params_json: *const c_char,
        callback: AsyncCallback,
        user_data: *mut c_void,
    );

    fn logos_core_register_event_listener(
        plugin_name: *const c_char,
        event_name: *const c_char,
        callback: AsyncCallback,
        user_data: *mut c_void,
    );
}

/// Shared state between the future and the FFI callback.
struct CallState {
    result: Option<Result<String, String>>,
    waker: Option<Waker>,
}

/// Future that resolves when the FFI callback fires.
struct CallFuture {
    state: Arc<Mutex<CallState>>,
}

impl Future for CallFuture {
    type Output = Result<String, String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// FFI trampoline: called from C when the async plugin method completes.
extern "C" fn call_trampoline(result: c_int, message: *const c_char, user_data: *mut c_void) {
    let state = unsafe { Arc::from_raw(user_data as *const Mutex<CallState>) };
    let msg = if message.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    let value = if result == 1 { Ok(msg) } else { Err(msg) };
    let mut guard = state.lock().unwrap();
    guard.result = Some(value);
    if let Some(waker) = guard.waker.take() {
        waker.wake();
    }
}

/// Call a Logos Core plugin method asynchronously.
///
/// `params` is a JSON array of `[{"name":"x","value":"y","type":"string"}, ...]`.
/// Returns `Ok(message)` if result==1, `Err(message)` otherwise.
pub async fn call_plugin_method(plugin: &str, method: &str, params: &str) -> Result<String, String> {
    let state = Arc::new(Mutex::new(CallState {
        result: None,
        waker: None,
    }));

    let plugin_c = CString::new(plugin).expect("plugin name contains null byte");
    let method_c = CString::new(method).expect("method name contains null byte");
    let params_c = CString::new(params).expect("params contain null byte");

    let state_ptr = Arc::into_raw(Arc::clone(&state)) as *mut c_void;

    unsafe {
        logos_core_call_plugin_method_async(
            plugin_c.as_ptr(),
            method_c.as_ptr(),
            params_c.as_ptr(),
            call_trampoline,
            state_ptr,
        );
    }

    CallFuture { state }.await
}

/// State for a persistent event listener (fires multiple times).
pub struct EventListenerState {
    pub sender: tokio::sync::mpsc::UnboundedSender<String>,
}

/// FFI trampoline for event listeners: forwards each event to an mpsc channel.
extern "C" fn event_trampoline(_result: c_int, message: *const c_char, user_data: *mut c_void) {
    let state = unsafe { &*(user_data as *const EventListenerState) };
    let msg = if message.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    let _ = state.sender.send(msg);
}

/// Register a persistent event listener on a Logos Core plugin.
///
/// Returns an `UnboundedReceiver<String>` that yields each event payload as JSON.
/// The returned `Box` must be kept alive for the listener to remain active.
pub fn register_event_listener(
    plugin: &str,
    event: &str,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<String>,
    Box<EventListenerState>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let state = Box::new(EventListenerState { sender: tx });
    let state_ptr: *const EventListenerState = &*state;

    let plugin_c = CString::new(plugin).expect("plugin name contains null byte");
    let event_c = CString::new(event).expect("event name contains null byte");

    unsafe {
        logos_core_register_event_listener(
            plugin_c.as_ptr(),
            event_c.as_ptr(),
            event_trampoline,
            state_ptr as *mut c_void,
        );
    }

    (rx, state)
}
