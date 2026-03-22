#![allow(clippy::missing_safety_doc)]
//! C FFI bridge for logos-messaging-a2a — enables Logos Core Qt module integration.
//!
//! Exposes WakuA2ANode operations via C-compatible functions.
//! The Qt module (C++) calls these functions to manage agents and messaging.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::Mutex;

use logos_messaging_a2a_core::Task;
use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

/// Tokio runtime shared across FFI calls.
static RT: Lazy<Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime")
});

/// Global node instance (single-node FFI for now).
static NODE: Lazy<Mutex<Option<WakuA2ANode<LogosMessagingTransport>>>> =
    Lazy::new(|| Mutex::new(None));

/// Helper: allocate a C string the caller must free with waku_a2a_free_string.
fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

/// Free a string returned by this library.
#[no_mangle]
pub unsafe extern "C" fn waku_a2a_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

/// Initialize a node with nwaku REST transport.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn waku_a2a_init(
    name: *const c_char,
    description: *const c_char,
    nwaku_url: *const c_char,
    encrypted: bool,
) -> i32 {
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    let desc = unsafe { CStr::from_ptr(description) }.to_string_lossy();
    let url = unsafe { CStr::from_ptr(nwaku_url) }.to_string_lossy();

    let transport = LogosMessagingTransport::new(&url);
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
pub unsafe extern "C" fn waku_a2a_pubkey() -> *mut c_char {
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
pub unsafe extern "C" fn waku_a2a_agent_card_json() -> *mut c_char {
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
pub unsafe extern "C" fn waku_a2a_announce() -> i32 {
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
pub unsafe extern "C" fn waku_a2a_discover() -> *mut c_char {
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
pub unsafe extern "C" fn waku_a2a_send_text(to_pubkey: *const c_char, text: *const c_char) -> i32 {
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
pub unsafe extern "C" fn waku_a2a_poll_tasks() -> *mut c_char {
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
pub unsafe extern "C" fn waku_a2a_respond(
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
pub unsafe extern "C" fn waku_a2a_shutdown() {
    if let Ok(mut guard) = NODE.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::ffi::CString;

    /// Helper: create a C string literal for test arguments.
    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// Ensure the global node is cleared before each test.
    fn reset_node() {
        unsafe {
            waku_a2a_shutdown();
        }
    }

    // ------------------------------------------------------------------
    // waku_a2a_free_string
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn free_string_null_does_not_crash() {
        unsafe {
            waku_a2a_free_string(ptr::null_mut());
        }
    }

    #[test]
    #[serial]
    fn free_string_valid_does_not_crash() {
        let s = to_c_string("hello from test");
        unsafe {
            waku_a2a_free_string(s);
        }
    }

    // ------------------------------------------------------------------
    // waku_a2a_pubkey — before init should be null
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn pubkey_returns_null_before_init() {
        reset_node();
        let pk = unsafe { waku_a2a_pubkey() };
        assert!(
            pk.is_null(),
            "pubkey should be null when no node is initialized"
        );
    }

    // ------------------------------------------------------------------
    // waku_a2a_agent_card_json — before init should be null
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn agent_card_json_returns_null_before_init() {
        reset_node();
        let json = unsafe { waku_a2a_agent_card_json() };
        assert!(
            json.is_null(),
            "agent_card_json should be null when no node is initialized"
        );
    }

    // ------------------------------------------------------------------
    // waku_a2a_init — plaintext mode
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn init_success_plaintext() {
        reset_node();
        let name = c("test-agent");
        let desc = c("A test agent");
        let url = c("http://127.0.0.1:8645");

        let ret = unsafe { waku_a2a_init(name.as_ptr(), desc.as_ptr(), url.as_ptr(), false) };
        assert_eq!(ret, 0, "init should return 0 on success");

        // pubkey should now be non-null and a valid hex string
        let pk = unsafe { waku_a2a_pubkey() };
        assert!(!pk.is_null(), "pubkey should be non-null after init");
        let pk_str = unsafe { CStr::from_ptr(pk) }.to_string_lossy().to_string();
        assert!(!pk_str.is_empty(), "pubkey string should not be empty");
        assert!(
            pk_str.chars().all(|c| c.is_ascii_hexdigit()),
            "pubkey should be hex: {pk_str}"
        );
        unsafe { waku_a2a_free_string(pk) };

        reset_node();
    }

    // ------------------------------------------------------------------
    // waku_a2a_init — encrypted mode
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn init_success_encrypted() {
        reset_node();
        let name = c("enc-agent");
        let desc = c("An encrypted test agent");
        let url = c("http://127.0.0.1:8645");

        let ret = unsafe { waku_a2a_init(name.as_ptr(), desc.as_ptr(), url.as_ptr(), true) };
        assert_eq!(ret, 0, "encrypted init should return 0");

        // pubkey should be valid hex
        let pk = unsafe { waku_a2a_pubkey() };
        assert!(!pk.is_null());
        let pk_str = unsafe { CStr::from_ptr(pk) }.to_string_lossy().to_string();
        assert!(pk_str.chars().all(|c| c.is_ascii_hexdigit()));
        unsafe { waku_a2a_free_string(pk) };

        // agent card should contain intro_bundle for encrypted mode
        let card_ptr = unsafe { waku_a2a_agent_card_json() };
        assert!(!card_ptr.is_null());
        let card_json = unsafe { CStr::from_ptr(card_ptr) }
            .to_string_lossy()
            .to_string();
        let v: serde_json::Value =
            serde_json::from_str(&card_json).expect("agent card should be valid JSON");
        assert!(
            v.get("intro_bundle").is_some(),
            "encrypted card should have intro_bundle"
        );
        unsafe { waku_a2a_free_string(card_ptr) };

        reset_node();
    }

    // ------------------------------------------------------------------
    // waku_a2a_agent_card_json — valid JSON with expected fields
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn agent_card_json_valid_after_init() {
        reset_node();
        let name = c("card-agent");
        let desc = c("Card test");
        let url = c("http://127.0.0.1:8645");

        let ret = unsafe { waku_a2a_init(name.as_ptr(), desc.as_ptr(), url.as_ptr(), false) };
        assert_eq!(ret, 0);

        let card_ptr = unsafe { waku_a2a_agent_card_json() };
        assert!(!card_ptr.is_null());
        let card_json = unsafe { CStr::from_ptr(card_ptr) }
            .to_string_lossy()
            .to_string();
        let v: serde_json::Value =
            serde_json::from_str(&card_json).expect("agent card should be valid JSON");

        assert_eq!(v["name"], "card-agent");
        assert_eq!(v["description"], "Card test");
        assert!(v["public_key"].is_string());
        assert!(v["capabilities"].is_array());
        unsafe { waku_a2a_free_string(card_ptr) };

        reset_node();
    }

    // ------------------------------------------------------------------
    // waku_a2a_shutdown — pubkey returns null afterwards
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn shutdown_clears_node() {
        reset_node();
        let name = c("shutdown-agent");
        let desc = c("Shutdown test");
        let url = c("http://127.0.0.1:8645");

        let ret = unsafe { waku_a2a_init(name.as_ptr(), desc.as_ptr(), url.as_ptr(), false) };
        assert_eq!(ret, 0);

        // pubkey is valid before shutdown
        let pk = unsafe { waku_a2a_pubkey() };
        assert!(!pk.is_null());
        unsafe { waku_a2a_free_string(pk) };

        // shutdown
        unsafe { waku_a2a_shutdown() };

        // pubkey should now be null
        let pk_after = unsafe { waku_a2a_pubkey() };
        assert!(pk_after.is_null(), "pubkey should be null after shutdown");
    }

    // ------------------------------------------------------------------
    // waku_a2a_respond — invalid JSON returns -1
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn respond_invalid_json_returns_error() {
        reset_node();
        let name = c("respond-agent");
        let desc = c("Respond test");
        let url = c("http://127.0.0.1:8645");

        let ret = unsafe { waku_a2a_init(name.as_ptr(), desc.as_ptr(), url.as_ptr(), false) };
        assert_eq!(ret, 0);

        let bad_json = c("this is not valid json");
        let result_text = c("some response");

        let ret = unsafe { waku_a2a_respond(bad_json.as_ptr(), result_text.as_ptr()) };
        assert_eq!(ret, -1, "respond with invalid JSON should return -1");

        reset_node();
    }

    // ------------------------------------------------------------------
    // Error paths: operations without a running nwaku node return -1/null
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn announce_without_node_returns_error() {
        reset_node();
        let ret = unsafe { waku_a2a_announce() };
        assert_eq!(ret, -1, "announce without node should return -1");
    }

    #[test]
    #[serial]
    fn discover_without_node_returns_null() {
        reset_node();
        let ret = unsafe { waku_a2a_discover() };
        assert!(ret.is_null(), "discover without node should return null");
    }

    #[test]
    #[serial]
    fn send_text_without_node_returns_error() {
        reset_node();
        let to = c("deadbeef");
        let text = c("hello");
        let ret = unsafe { waku_a2a_send_text(to.as_ptr(), text.as_ptr()) };
        assert_eq!(ret, -1, "send_text without node should return -1");
    }

    #[test]
    #[serial]
    fn poll_tasks_without_node_returns_null() {
        reset_node();
        let ret = unsafe { waku_a2a_poll_tasks() };
        assert!(ret.is_null(), "poll_tasks without node should return null");
    }

    #[test]
    #[serial]
    fn respond_without_node_returns_error() {
        reset_node();
        let task_json = c("{}");
        let result_text = c("reply");
        let ret = unsafe { waku_a2a_respond(task_json.as_ptr(), result_text.as_ptr()) };
        assert_eq!(ret, -1, "respond without node should return -1");
    }

    // ------------------------------------------------------------------
    // Re-initialization: init can be called again after shutdown
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn reinit_after_shutdown() {
        reset_node();
        let name = c("agent-v1");
        let desc = c("First init");
        let url = c("http://127.0.0.1:8645");

        let ret = unsafe { waku_a2a_init(name.as_ptr(), desc.as_ptr(), url.as_ptr(), false) };
        assert_eq!(ret, 0);

        let pk1 = unsafe { waku_a2a_pubkey() };
        assert!(!pk1.is_null());
        let pk1_str = unsafe { CStr::from_ptr(pk1) }.to_string_lossy().to_string();
        unsafe { waku_a2a_free_string(pk1) };

        unsafe { waku_a2a_shutdown() };

        // Re-init with new identity
        let name2 = c("agent-v2");
        let desc2 = c("Second init");
        let ret = unsafe { waku_a2a_init(name2.as_ptr(), desc2.as_ptr(), url.as_ptr(), false) };
        assert_eq!(ret, 0);

        let pk2 = unsafe { waku_a2a_pubkey() };
        assert!(!pk2.is_null());
        let pk2_str = unsafe { CStr::from_ptr(pk2) }.to_string_lossy().to_string();
        unsafe { waku_a2a_free_string(pk2) };

        // New node should have a different key
        assert_ne!(
            pk1_str, pk2_str,
            "re-initialized node should have a new keypair"
        );

        reset_node();
    }
}
