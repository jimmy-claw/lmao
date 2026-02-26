//! C FFI bridge for waku-a2a — enables Logos Core Qt module integration.
//!
//! Exposes WakuA2ANode operations via C-compatible functions.
//! The Qt module (C++) calls these functions to manage agents and messaging.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use tokio::runtime::Runtime;
use waku_a2a_core::Task;
use waku_a2a_node::WakuA2ANode;
use waku_a2a_transport::nwaku_rest::NwakuRestTransport;

/// Tokio runtime shared across FFI calls.
static RT: Lazy<Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime")
});

/// Global node instance (single-node FFI for now).
static NODE: Lazy<Mutex<Option<WakuA2ANode<NwakuRestTransport>>>> =
    Lazy::new(|| Mutex::new(None));

/// Helper: allocate a C string the caller must free with waku_a2a_free_string.
fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

/// Free a string returned by this library.
#[no_mangle]
pub extern "C" fn waku_a2a_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)); }
    }
}

/// Initialize a node with nwaku REST transport.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn waku_a2a_init(
    name: *const c_char,
    description: *const c_char,
    nwaku_url: *const c_char,
    encrypted: bool,
) -> i32 {
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    let desc = unsafe { CStr::from_ptr(description) }.to_string_lossy();
    let url = unsafe { CStr::from_ptr(nwaku_url) }.to_string_lossy();

    let transport = NwakuRestTransport::new(&url);
    let node = if encrypted {
        WakuA2ANode::new_encrypted(&name, &desc, vec!["text".into()], transport)
    } else {
        WakuA2ANode::new(&name, &desc, vec!["text".into()], transport)
    };

    match NODE.lock() {
        Ok(mut guard) => {
            *guard = Some(node);
            0
        }
        Err(_) => -1,
    }
}

/// Get this node's public key (hex). Caller must free the result.
#[no_mangle]
pub extern "C" fn waku_a2a_pubkey() -> *mut c_char {
    match NODE.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(node) => to_c_string(node.pubkey()),
            None => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Get the agent card as JSON. Caller must free the result.
#[no_mangle]
pub extern "C" fn waku_a2a_agent_card_json() -> *mut c_char {
    match NODE.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(node) => match serde_json::to_string(&node.card) {
                Ok(json) => to_c_string(&json),
                Err(_) => ptr::null_mut(),
            },
            None => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Announce this agent on the discovery topic.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn waku_a2a_announce() -> i32 {
    match NODE.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(node) => match RT.block_on(node.announce()) {
                Ok(_) => 0,
                Err(_) => -1,
            },
            None => -1,
        },
        Err(_) => -1,
    }
}

/// Discover agents. Returns JSON array of AgentCards. Caller must free the result.
#[no_mangle]
pub extern "C" fn waku_a2a_discover() -> *mut c_char {
    match NODE.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(node) => match RT.block_on(node.discover()) {
                Ok(cards) => match serde_json::to_string(&cards) {
                    Ok(json) => to_c_string(&json),
                    Err(_) => ptr::null_mut(),
                },
                Err(_) => ptr::null_mut(),
            },
            None => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Send a text message to another agent. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn waku_a2a_send_text(
    to_pubkey: *const c_char,
    text: *const c_char,
) -> i32 {
    let to = unsafe { CStr::from_ptr(to_pubkey) }.to_string_lossy();
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();

    match NODE.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(node) => match RT.block_on(node.send_text(&to, &text)) {
                Ok(_) => 0,
                Err(_) => -1,
            },
            None => -1,
        },
        Err(_) => -1,
    }
}

/// Poll for incoming tasks. Returns JSON array of Tasks. Caller must free the result.
#[no_mangle]
pub extern "C" fn waku_a2a_poll_tasks() -> *mut c_char {
    match NODE.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(node) => match RT.block_on(node.poll_tasks()) {
                Ok(tasks) => match serde_json::to_string(&tasks) {
                    Ok(json) => to_c_string(&json),
                    Err(_) => ptr::null_mut(),
                },
                Err(_) => ptr::null_mut(),
            },
            None => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Respond to a task. task_json is the original task JSON, result_text is the response.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn waku_a2a_respond(
    task_json: *const c_char,
    result_text: *const c_char,
) -> i32 {
    let task_str = unsafe { CStr::from_ptr(task_json) }.to_string_lossy();
    let result_text = unsafe { CStr::from_ptr(result_text) }.to_string_lossy();

    let task: Task = match serde_json::from_str(&task_str) {
        Ok(t) => t,
        Err(_) => return -1,
    };

    match NODE.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(node) => match RT.block_on(node.respond(&task, &result_text)) {
                Ok(_) => 0,
                Err(_) => -1,
            },
            None => -1,
        },
        Err(_) => -1,
    }
}

/// Shutdown and release the node.
#[no_mangle]
pub extern "C" fn waku_a2a_shutdown() {
    if let Ok(mut guard) = NODE.lock() {
        *guard = None;
    }
}
