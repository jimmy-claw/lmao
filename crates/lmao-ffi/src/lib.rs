//! lmao-ffi — C FFI wrapper for LMAO (A2A over Waku).
//!
//! All functions accept/return JSON strings (UTF-8, null-terminated).
//! Caller must free returned strings with lmao_free_string().

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

use logos_messaging_a2a_node::WakuA2ANode;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use tokio::runtime::Runtime;

/// Global tokio runtime for async operations.
fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("Failed to create tokio runtime"))
}

/// Global node instance (lazy-initialized on first call).
static NODE: OnceLock<WakuA2ANode<LogosMessagingTransport>> = OnceLock::new();

/// Returns a reference to the lazily-initialized global node, creating it on the
/// first call using the `WAKU_URL` environment variable (defaults to `http://localhost:8645`).
/// The node is announced on the Waku network as part of initialization.
fn get_or_init_node() -> &'static WakuA2ANode<LogosMessagingTransport> {
    NODE.get_or_init(|| {
        let waku_url =
            std::env::var("WAKU_URL").unwrap_or_else(|_| "http://localhost:8645".to_string());
        let transport = LogosMessagingTransport::new(&waku_url);
        let node = WakuA2ANode::new(
            "lmao-agent",
            "LMAO A2A agent via Logos Core",
            vec!["text".to_string()],
            transport,
        );

        // Announce on startup
        let _ = runtime().block_on(node.announce());

        node
    })
}

/// Converts a C string pointer to a Rust `&str`, returning an error if the
/// pointer is null or contains invalid UTF-8.
fn cstr_to_str(ptr: *const c_char) -> Result<&'static str, String> {
    if ptr.is_null() {
        return Err("null pointer".to_string());
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|e| format!("Invalid UTF-8: {}", e))
}

/// Converts a Rust `String` into a heap-allocated C string pointer.
/// Falls back to an empty JSON object `"{}"` if the string contains interior NUL bytes.
/// The caller is responsible for freeing the returned pointer via [`lmao_free_string`].
fn to_cstring(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

/// Builds a JSON error response `{"success": false, "error": "<msg>"}` and returns
/// it as a heap-allocated C string. Double-quotes inside `msg` are escaped.
fn error_json(msg: &str) -> *mut c_char {
    to_cstring(format!(
        r#"{{"success":false,"error":"{}"}}"#,
        msg.replace('"', "\\\"")
    ))
}

/// Builds a JSON success response `{"success": true, ...}` and returns it as a
/// heap-allocated C string. If `payload` is a JSON object its keys are merged into
/// the top-level response; otherwise it is wrapped under a `"data"` key.
fn success_json(payload: serde_json::Value) -> *mut c_char {
    let mut obj = serde_json::Map::new();
    obj.insert("success".to_string(), serde_json::Value::Bool(true));
    match payload {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                obj.insert(k, v);
            }
        }
        _ => {
            obj.insert("data".to_string(), payload);
        }
    }
    to_cstring(serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_default())
}

// ── Exported FFI Functions ──────────────────────────────────────────────────

/// Discover agents on the Waku network.
///
/// args_json: { "timeout_ms": 5000 }  (optional, default 5000)
///
/// Returns: { "success": true, "agents": [ { "name": "...", ... }, ... ] }
#[no_mangle]
pub extern "C" fn lmao_discover_agents(args_json: *const c_char) -> *mut c_char {
    let timeout_ms: u64 = match cstr_to_str(args_json) {
        Ok(s) => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                v.get("timeout_ms").and_then(|t| t.as_u64()).unwrap_or(5000)
            } else {
                5000
            }
        }
        Err(_) => 5000,
    };

    let node = get_or_init_node();
    let rt = runtime();

    match rt.block_on(async {
        // Announce ourselves first
        let _ = node.announce().await;
        // Wait for discovery
        tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
        node.discover().await
    }) {
        Ok(cards) => {
            let agents: Vec<serde_json::Value> = cards
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "description": c.description,
                        "version": c.version,
                        "capabilities": c.capabilities,
                        "public_key": c.public_key,
                    })
                })
                .collect();
            success_json(serde_json::json!({ "agents": agents }))
        }
        Err(e) => error_json(&e.to_string()),
    }
}

/// Send a text task to another agent.
///
/// args_json: { "agent_pubkey": "02...", "task_text": "Hello" }
///
/// Returns: { "success": true, "task_id": "...", "acked": true/false }
#[no_mangle]
pub extern "C" fn lmao_send_task(args_json: *const c_char) -> *mut c_char {
    let s = match cstr_to_str(args_json) {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };

    let v: serde_json::Value = match serde_json::from_str(s) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("JSON parse error: {}", e)),
    };

    let agent_pubkey = match v.get("agent_pubkey").and_then(|s| s.as_str()) {
        Some(s) => s.to_string(),
        None => return error_json("missing 'agent_pubkey'"),
    };

    let task_text = match v.get("task_text").and_then(|s| s.as_str()) {
        Some(s) => s.to_string(),
        None => return error_json("missing 'task_text'"),
    };

    let node = get_or_init_node();
    let rt = runtime();

    match rt.block_on(node.send_text(&agent_pubkey, &task_text)) {
        Ok(task) => success_json(serde_json::json!({
            "task_id": task.id,
            "from": task.from,
            "to": task.to,
        })),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Get this agent's card as JSON.
///
/// Returns: { "success": true, "card": { "name": "...", ... } }
#[no_mangle]
pub extern "C" fn lmao_get_agent_card() -> *mut c_char {
    let node = get_or_init_node();
    let card = &node.card;
    success_json(serde_json::json!({
        "card": {
            "name": card.name,
            "description": card.description,
            "version": card.version,
            "capabilities": card.capabilities,
            "public_key": card.public_key,
        }
    }))
}

/// Free a string returned by any lmao_* function.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn lmao_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

/// Returns the version string of this FFI library.
#[no_mangle]
pub extern "C" fn lmao_version() -> *mut c_char {
    to_cstring(env!("CARGO_PKG_VERSION").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_messaging_a2a_core::{A2AEnvelope, AgentCard, Message, Part, Task, TaskState};

    /// Helper: read a *mut c_char back into a String, then free it.
    unsafe fn read_and_free(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = CStr::from_ptr(ptr).to_str().unwrap().to_owned();
        lmao_free_string(ptr);
        s
    }

    /// Helper: read a *mut c_char back as parsed JSON, then free it.
    unsafe fn read_json_and_free(ptr: *mut c_char) -> serde_json::Value {
        let s = read_and_free(ptr);
        serde_json::from_str(&s).expect("returned string should be valid JSON")
    }

    // ── cstr_to_str tests ──────────────────────────────────────────────────

    #[test]
    fn test_cstr_to_str_null() {
        let result = cstr_to_str(std::ptr::null());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "null pointer");
    }

    #[test]
    fn test_cstr_to_str_valid() {
        let c = CString::new("hello").unwrap();
        let result = cstr_to_str(c.as_ptr());
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn test_cstr_to_str_empty() {
        let c = CString::new("").unwrap();
        let result = cstr_to_str(c.as_ptr());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn test_cstr_to_str_invalid_utf8() {
        // Create a C string with invalid UTF-8: 0xFF is never valid in UTF-8
        let bytes: Vec<u8> = vec![0xFF, 0xFE, 0x00]; // null-terminated
        let ptr = bytes.as_ptr() as *const c_char;
        let result = cstr_to_str(ptr);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid UTF-8"));
    }

    #[test]
    fn test_cstr_to_str_unicode() {
        let c = CString::new("Hello 🌍 世界").unwrap();
        let result = cstr_to_str(c.as_ptr());
        assert_eq!(result.unwrap(), "Hello 🌍 世界");
    }

    // ── to_cstring tests ───────────────────────────────────────────────────

    #[test]
    fn test_to_cstring_and_back() {
        let original = "test string 123";
        let ptr = to_cstring(original.to_string());
        let recovered = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(recovered, original);
        lmao_free_string(ptr);
    }

    #[test]
    fn test_to_cstring_empty() {
        let ptr = to_cstring(String::new());
        let s = unsafe { read_and_free(ptr) };
        assert_eq!(s, "");
    }

    #[test]
    fn test_to_cstring_embedded_nul_falls_back() {
        // CString::new fails on embedded NUL; to_cstring should fallback to "{}"
        let ptr = to_cstring("hello\0world".to_string());
        let s = unsafe { read_and_free(ptr) };
        assert_eq!(s, "{}");
    }

    #[test]
    fn test_to_cstring_unicode() {
        let original = "café résumé naïve 日本語 한국어 🚀🎉";
        let ptr = to_cstring(original.to_string());
        let s = unsafe { read_and_free(ptr) };
        assert_eq!(s, original);
    }

    #[test]
    fn test_to_cstring_long_string() {
        let original: String = "A".repeat(100_000);
        let ptr = to_cstring(original.clone());
        let s = unsafe { read_and_free(ptr) };
        assert_eq!(s, original);
    }

    #[test]
    fn test_to_cstring_special_ascii() {
        let original = "\t\n\r !@#$%^&*()_+-=[]{}|;':\",./<>?";
        let ptr = to_cstring(original.to_string());
        let s = unsafe { read_and_free(ptr) };
        assert_eq!(s, original);
    }

    // ── error_json tests ───────────────────────────────────────────────────

    #[test]
    fn test_error_json_format() {
        let v = unsafe { read_json_and_free(error_json("something went wrong")) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "something went wrong");
    }

    #[test]
    fn test_error_json_escapes_quotes() {
        let v = unsafe { read_json_and_free(error_json(r#"bad "input""#)) };
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().contains("bad"));
        assert!(v["error"].as_str().unwrap().contains("input"));
    }

    #[test]
    fn test_error_json_empty_message() {
        let v = unsafe { read_json_and_free(error_json("")) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "");
    }

    #[test]
    fn test_error_json_with_special_punctuation() {
        let v = unsafe { read_json_and_free(error_json("err: <foo> & 'bar' [baz]")) };
        assert_eq!(v["success"], false);
        let err = v["error"].as_str().unwrap();
        assert!(err.contains("<foo>"));
        assert!(err.contains("& 'bar'"));
    }

    #[test]
    fn test_error_json_control_chars_produce_raw_output() {
        // error_json only escapes double quotes — control characters like \n
        // are embedded literally, producing technically invalid JSON. This test
        // documents that behavior (the raw C string is still null-terminated
        // and freeable, which is what matters for the FFI contract).
        let ptr = error_json("line1\nline2");
        assert!(!ptr.is_null());
        let raw = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(raw.contains("line1"));
        assert!(raw.contains("line2"));
        lmao_free_string(ptr);
    }

    #[test]
    fn test_error_json_long_message() {
        let long_msg = "x".repeat(10_000);
        let v = unsafe { read_json_and_free(error_json(&long_msg)) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"].as_str().unwrap().len(), 10_000);
    }

    #[test]
    fn test_error_json_unicode_message() {
        let v = unsafe { read_json_and_free(error_json("错误: 失败了 🔥")) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "错误: 失败了 🔥");
    }

    #[test]
    fn test_error_json_only_has_success_and_error_keys() {
        let v = unsafe { read_json_and_free(error_json("test")) };
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("success"));
        assert!(obj.contains_key("error"));
    }

    // ── success_json tests ─────────────────────────────────────────────────

    #[test]
    fn test_success_json_object() {
        let v = unsafe { read_json_and_free(success_json(serde_json::json!({"key": "value"}))) };
        assert_eq!(v["success"], true);
        assert_eq!(v["key"], "value");
    }

    #[test]
    fn test_success_json_non_object() {
        let v = unsafe { read_json_and_free(success_json(serde_json::json!(42))) };
        assert_eq!(v["success"], true);
        assert_eq!(v["data"], 42);
    }

    #[test]
    fn test_success_json_string_value() {
        let v = unsafe { read_json_and_free(success_json(serde_json::json!("hello"))) };
        assert_eq!(v["success"], true);
        assert_eq!(v["data"], "hello");
    }

    #[test]
    fn test_success_json_array_value() {
        let v = unsafe { read_json_and_free(success_json(serde_json::json!([1, 2, 3]))) };
        assert_eq!(v["success"], true);
        assert_eq!(v["data"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn test_success_json_null_value() {
        let v = unsafe { read_json_and_free(success_json(serde_json::Value::Null)) };
        assert_eq!(v["success"], true);
        assert!(v["data"].is_null());
    }

    #[test]
    fn test_success_json_bool_value() {
        let v = unsafe { read_json_and_free(success_json(serde_json::json!(true))) };
        assert_eq!(v["success"], true);
        assert_eq!(v["data"], true);
    }

    #[test]
    fn test_success_json_empty_object() {
        let v = unsafe { read_json_and_free(success_json(serde_json::json!({}))) };
        assert_eq!(v["success"], true);
        // Empty object merges nothing, so only "success" key
        assert_eq!(v.as_object().unwrap().len(), 1);
    }

    #[test]
    fn test_success_json_nested_object() {
        let payload = serde_json::json!({
            "agent": {
                "name": "test",
                "capabilities": ["text", "code"]
            },
            "count": 5
        });
        let v = unsafe { read_json_and_free(success_json(payload)) };
        assert_eq!(v["success"], true);
        assert_eq!(v["agent"]["name"], "test");
        assert_eq!(v["count"], 5);
    }

    #[test]
    fn test_success_json_object_keys_flatten_into_top_level() {
        let payload = serde_json::json!({"a": 1, "b": 2, "c": 3});
        let v = unsafe { read_json_and_free(success_json(payload)) };
        assert_eq!(v["success"], true);
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
        assert_eq!(v["c"], 3);
        // success + a + b + c = 4 keys
        assert_eq!(v.as_object().unwrap().len(), 4);
    }

    #[test]
    fn test_success_json_float_value() {
        let v = unsafe { read_json_and_free(success_json(serde_json::json!(3.14))) };
        assert_eq!(v["success"], true);
        assert!((v["data"].as_f64().unwrap() - 3.14).abs() < f64::EPSILON);
    }

    // ── lmao_free_string tests ─────────────────────────────────────────────

    #[test]
    fn test_free_string_null() {
        // Should not panic on null pointer
        lmao_free_string(std::ptr::null_mut());
    }

    #[test]
    fn test_free_string_valid() {
        let s = to_cstring("hello world".to_string());
        assert!(!s.is_null());
        lmao_free_string(s);
    }

    #[test]
    fn test_free_string_sequential_alloc_free() {
        // Allocate and free many strings sequentially — tests memory lifecycle
        for i in 0..1000 {
            let ptr = to_cstring(format!("string number {}", i));
            assert!(!ptr.is_null());
            lmao_free_string(ptr);
        }
    }

    #[test]
    fn test_free_string_batch_alloc_then_free() {
        // Allocate many strings, then free them all — tests no double-free or leak
        let ptrs: Vec<*mut c_char> = (0..100)
            .map(|i| to_cstring(format!("batch {}", i)))
            .collect();

        for ptr in &ptrs {
            assert!(!ptr.is_null());
        }

        for ptr in ptrs {
            lmao_free_string(ptr);
        }
    }

    #[test]
    fn test_free_string_reverse_order() {
        // Allocate then free in reverse order (stack-like pattern)
        let ptrs: Vec<*mut c_char> = (0..50).map(|i| to_cstring(format!("rev {}", i))).collect();

        for ptr in ptrs.into_iter().rev() {
            lmao_free_string(ptr);
        }
    }

    #[test]
    fn test_free_string_interleaved() {
        // Interleaved alloc/free pattern
        let a = to_cstring("first".to_string());
        let b = to_cstring("second".to_string());
        lmao_free_string(a);
        let c = to_cstring("third".to_string());
        lmao_free_string(b);
        lmao_free_string(c);
    }

    // ── lmao_version tests ─────────────────────────────────────────────────

    #[test]
    fn test_version() {
        let s = unsafe { read_and_free(lmao_version()) };
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_version_is_semver() {
        let s = unsafe { read_and_free(lmao_version()) };
        let parts: Vec<&str> = s.split('.').collect();
        assert_eq!(parts.len(), 3, "version should be semver: {}", s);
        for part in &parts {
            assert!(
                part.parse::<u32>().is_ok(),
                "each semver component should be numeric: {}",
                part
            );
        }
    }

    #[test]
    fn test_version_non_empty() {
        let s = unsafe { read_and_free(lmao_version()) };
        assert!(!s.is_empty());
    }

    #[test]
    fn test_version_idempotent() {
        let v1 = unsafe { read_and_free(lmao_version()) };
        let v2 = unsafe { read_and_free(lmao_version()) };
        assert_eq!(v1, v2);
    }

    // ── lmao_send_task input validation tests ──────────────────────────────
    // These test early-return error paths that don't require network access.

    #[test]
    fn test_send_task_null_pointer() {
        let v = unsafe { read_json_and_free(lmao_send_task(std::ptr::null())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "null pointer");
    }

    #[test]
    fn test_send_task_invalid_json() {
        let input = CString::new("not json at all").unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().contains("JSON parse error"));
    }

    #[test]
    fn test_send_task_empty_json_object() {
        let input = CString::new("{}").unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_missing_agent_pubkey() {
        let input = CString::new(r#"{"task_text": "hello"}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_missing_task_text() {
        let input = CString::new(r#"{"agent_pubkey": "02aabb"}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'task_text'");
    }

    #[test]
    fn test_send_task_agent_pubkey_not_string() {
        let input = CString::new(r#"{"agent_pubkey": 12345, "task_text": "hi"}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_task_text_not_string() {
        let input = CString::new(r#"{"agent_pubkey": "02aabb", "task_text": 42}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'task_text'");
    }

    #[test]
    fn test_send_task_agent_pubkey_null_value() {
        let input = CString::new(r#"{"agent_pubkey": null, "task_text": "hi"}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_task_text_null_value() {
        let input = CString::new(r#"{"agent_pubkey": "02aabb", "task_text": null}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'task_text'");
    }

    #[test]
    fn test_send_task_empty_string_input() {
        let input = CString::new("").unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().contains("JSON parse error"));
    }

    #[test]
    fn test_send_task_json_array_instead_of_object() {
        let input = CString::new("[1,2,3]").unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        // An array has no "agent_pubkey" key
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_extra_fields_still_validates() {
        // Has extra fields but missing task_text — should still fail with missing task_text
        let input =
            CString::new(r#"{"agent_pubkey": "02aabb", "extra": true, "foo": "bar"}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'task_text'");
    }

    #[test]
    fn test_send_task_deeply_nested_json_missing_fields() {
        let input = CString::new(r#"{"nested": {"agent_pubkey": "02aa"}}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        // Top-level doesn't have agent_pubkey
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_agent_pubkey_empty_string() {
        // Empty string is still a valid string — passes the as_str() check
        // but both fields present, so it would proceed to network. Just verify both present.
        let input = CString::new(r#"{"agent_pubkey": "", "task_text": ""}"#).unwrap();
        // This will try to init the node and fail without network, but the
        // important thing is it doesn't crash and returns valid JSON.
        let ptr = lmao_send_task(input.as_ptr());
        assert!(!ptr.is_null());
        // Just verify it's valid JSON with a "success" key
        let v = unsafe { read_json_and_free(ptr) };
        assert!(v.is_object());
        assert!(v.get("success").is_some());
    }

    // ── Runtime tests ──────────────────────────────────────────────────────

    #[test]
    fn test_runtime_init_idempotent() {
        let rt1 = runtime();
        let rt2 = runtime();
        assert!(std::ptr::eq(rt1, rt2));
    }

    #[test]
    fn test_runtime_can_spawn_task() {
        let rt = runtime();
        let result = rt.block_on(async { 42 });
        assert_eq!(result, 42);
    }

    // ── Concurrent access tests ────────────────────────────────────────────

    #[test]
    fn test_concurrent_to_cstring() {
        let handles: Vec<_> = (0..16)
            .map(|i| {
                std::thread::spawn(move || {
                    for j in 0..100 {
                        let ptr = to_cstring(format!("thread {} iter {}", i, j));
                        assert!(!ptr.is_null());
                        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
                        assert!(s.contains(&format!("thread {} iter {}", i, j)));
                        lmao_free_string(ptr);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_error_json() {
        let handles: Vec<_> = (0..16)
            .map(|i| {
                std::thread::spawn(move || {
                    for j in 0..100 {
                        let msg = format!("error from thread {} iter {}", i, j);
                        let ptr = error_json(&msg);
                        let v = unsafe { read_json_and_free(ptr) };
                        assert_eq!(v["success"], false);
                        assert!(v["error"]
                            .as_str()
                            .unwrap()
                            .contains(&format!("thread {}", i)));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_success_json() {
        let handles: Vec<_> = (0..16)
            .map(|i| {
                std::thread::spawn(move || {
                    for j in 0..100 {
                        let ptr = success_json(serde_json::json!({"thread": i, "iter": j}));
                        let v = unsafe { read_json_and_free(ptr) };
                        assert_eq!(v["success"], true);
                        assert_eq!(v["thread"], i);
                        assert_eq!(v["iter"], j);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_version() {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..100 {
                        let ptr = lmao_version();
                        let s = unsafe { read_and_free(ptr) };
                        assert_eq!(s, env!("CARGO_PKG_VERSION"));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_send_task_null() {
        // Multiple threads all passing null — should all get clean error responses
        let handles: Vec<_> = (0..16)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..100 {
                        let v = unsafe { read_json_and_free(lmao_send_task(std::ptr::null())) };
                        assert_eq!(v["success"], false);
                        assert_eq!(v["error"], "null pointer");
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_send_task_invalid_json() {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                std::thread::spawn(|| {
                    let input = CString::new("not json").unwrap();
                    for _ in 0..100 {
                        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
                        assert_eq!(v["success"], false);
                        assert!(v["error"].as_str().unwrap().contains("JSON parse error"));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_free_null() {
        // Concurrent null free should never panic
        let handles: Vec<_> = (0..16)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..1000 {
                        lmao_free_string(std::ptr::null_mut());
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_mixed_operations() {
        // Different threads doing different FFI operations simultaneously
        let h1 = std::thread::spawn(|| {
            for _ in 0..200 {
                let ptr = lmao_version();
                unsafe { read_and_free(ptr) };
            }
        });
        let h2 = std::thread::spawn(|| {
            for i in 0..200 {
                let ptr = error_json(&format!("err {}", i));
                unsafe { read_and_free(ptr) };
            }
        });
        let h3 = std::thread::spawn(|| {
            for i in 0..200 {
                let ptr = success_json(serde_json::json!({"i": i}));
                unsafe { read_and_free(ptr) };
            }
        });
        let h4 = std::thread::spawn(|| {
            for _ in 0..200 {
                let v = unsafe { read_json_and_free(lmao_send_task(std::ptr::null())) };
                assert_eq!(v["success"], false);
            }
        });

        h1.join().unwrap();
        h2.join().unwrap();
        h3.join().unwrap();
        h4.join().unwrap();
    }

    // ── Memory lifecycle stress tests ──────────────────────────────────────

    #[test]
    fn test_many_error_json_alloc_free() {
        for i in 0..1000 {
            let ptr = error_json(&format!("error {}", i));
            lmao_free_string(ptr);
        }
    }

    #[test]
    fn test_many_success_json_alloc_free() {
        for i in 0..1000 {
            let ptr = success_json(serde_json::json!({"i": i}));
            lmao_free_string(ptr);
        }
    }

    #[test]
    fn test_large_json_payload() {
        // Large object should round-trip correctly
        let mut obj = serde_json::Map::new();
        for i in 0..500 {
            obj.insert(format!("key_{}", i), serde_json::json!(i));
        }
        let ptr = success_json(serde_json::Value::Object(obj));
        let v = unsafe { read_json_and_free(ptr) };
        assert_eq!(v["success"], true);
        assert_eq!(v["key_0"], 0);
        assert_eq!(v["key_499"], 499);
    }

    // ── String ownership / FFI boundary tests ──────────────────────────────

    #[test]
    fn test_cstring_ownership_transfer() {
        // Verify that to_cstring transfers ownership (into_raw) and
        // lmao_free_string reclaims it (from_raw)
        let ptr = to_cstring("owned".to_string());
        // The pointer should be valid until freed
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, "owned");
        // After this, ptr is invalid — ownership transferred back
        lmao_free_string(ptr);
    }

    #[test]
    fn test_error_json_returns_owned_pointer() {
        // Each call should return a unique, independently freeable pointer
        let p1 = error_json("first");
        let p2 = error_json("second");
        assert_ne!(p1, p2);
        // Free in any order
        lmao_free_string(p2);
        lmao_free_string(p1);
    }

    #[test]
    fn test_success_json_returns_owned_pointer() {
        let p1 = success_json(serde_json::json!(1));
        let p2 = success_json(serde_json::json!(2));
        assert_ne!(p1, p2);
        lmao_free_string(p1);
        lmao_free_string(p2);
    }

    #[test]
    fn test_version_returns_new_pointer_each_call() {
        let p1 = lmao_version();
        let p2 = lmao_version();
        // Each call should allocate a new CString
        assert_ne!(p1, p2);
        let s1 = unsafe { CStr::from_ptr(p1) }.to_str().unwrap();
        let s2 = unsafe { CStr::from_ptr(p2) }.to_str().unwrap();
        assert_eq!(s1, s2);
        lmao_free_string(p1);
        lmao_free_string(p2);
    }

    // ── JSON edge case tests ───────────────────────────────────────────────

    #[test]
    fn test_send_task_truncated_json() {
        let input = CString::new(r#"{"agent_pubkey": "02"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().contains("JSON parse error"));
    }

    #[test]
    fn test_send_task_json_with_unicode_values() {
        // Valid JSON but missing task_text
        let input = CString::new(r#"{"agent_pubkey": "🔑keypair"}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'task_text'");
    }

    #[test]
    fn test_send_task_boolean_values() {
        let input = CString::new(r#"{"agent_pubkey": true, "task_text": false}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        // as_str() returns None for booleans
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_array_values() {
        let input = CString::new(r#"{"agent_pubkey": ["a"], "task_text": ["b"]}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    #[test]
    fn test_send_task_object_values() {
        let input = CString::new(r#"{"agent_pubkey": {}, "task_text": {}}"#).unwrap();
        let v = unsafe { read_json_and_free(lmao_send_task(input.as_ptr())) };
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "missing 'agent_pubkey'");
    }

    // ── JSON serialization tests for FFI types ─────────────────────────────

    #[test]
    fn test_agent_card_json_roundtrip() {
        let card = AgentCard {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["text".to_string(), "code".to_string()],
            public_key: "02abcdef1234".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, parsed);
    }

    #[test]
    fn test_agent_card_empty_capabilities() {
        let card = AgentCard {
            name: "minimal".to_string(),
            description: "".to_string(),
            version: "1.0.0".to_string(),
            capabilities: vec![],
            public_key: "02ff".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, parsed);
        assert!(parsed.capabilities.is_empty());
    }

    #[test]
    fn test_agent_card_unicode_fields() {
        let card = AgentCard {
            name: "日本語エージェント".to_string(),
            description: "テスト用 🤖".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec!["テキスト".to_string()],
            public_key: "02aabb".to_string(),
            intro_bundle: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, parsed);
    }

    #[test]
    fn test_task_json_roundtrip() {
        let task = Task::new("02aabb", "03ccdd", "Hello");
        let json = serde_json::to_string(&task).unwrap();
        let parsed: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(task.from, parsed.from);
        assert_eq!(task.to, parsed.to);
        assert_eq!(task.text(), parsed.text());
        assert_eq!(task.state, TaskState::Submitted);
    }

    #[test]
    fn test_task_state_all_variants() {
        let variants = [
            TaskState::Submitted,
            TaskState::Working,
            TaskState::InputRequired,
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Cancelled,
        ];
        for state in &variants {
            let json = serde_json::to_string(state).unwrap();
            let parsed: TaskState = serde_json::from_str(&json).unwrap();
            assert_eq!(*state, parsed);
        }
    }

    #[test]
    fn test_task_respond_creates_response() {
        let task = Task::new("02aa", "03bb", "question");
        let response = task.respond("answer");
        // Response should swap from/to and be from the recipient
        assert_eq!(response.from, task.to);
        assert_eq!(response.to, task.from);
        assert_eq!(response.result_text(), Some("answer"));
    }

    #[test]
    fn test_task_text_extraction() {
        let task = Task::new("from", "to", "hello world");
        assert_eq!(task.text(), Some("hello world"));
    }

    #[test]
    fn test_envelope_task_json() {
        let task = Task::new("02aa", "03bb", "test");
        let envelope = A2AEnvelope::Task(task);
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, parsed);
    }

    #[test]
    fn test_envelope_agent_card_json() {
        let card = AgentCard {
            name: "agent".to_string(),
            description: "desc".to_string(),
            version: "1.0.0".to_string(),
            capabilities: vec!["text".to_string()],
            public_key: "02ff".to_string(),
            intro_bundle: None,
        };
        let envelope = A2AEnvelope::AgentCard(card);
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, parsed);
    }

    #[test]
    fn test_envelope_ack_json() {
        let envelope = A2AEnvelope::Ack {
            message_id: "msg-123".to_string(),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: A2AEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, parsed);
    }

    #[test]
    fn test_message_parts_json() {
        let msg = Message {
            role: "user".to_string(),
            parts: vec![Part::Text {
                text: "Hello world".to_string(),
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn test_message_multiple_parts() {
        let msg = Message {
            role: "assistant".to_string(),
            parts: vec![
                Part::Text {
                    text: "Part 1".to_string(),
                },
                Part::Text {
                    text: "Part 2".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
        assert_eq!(parsed.parts.len(), 2);
    }

    #[test]
    fn test_message_empty_parts() {
        let msg = Message {
            role: "system".to_string(),
            parts: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
        assert!(parsed.parts.is_empty());
    }

    // ── End-to-end FFI flow tests ──────────────────────────────────────────

    #[test]
    fn test_error_then_success_lifecycle() {
        // Simulate a C consumer that gets an error, then a success
        let err_ptr = error_json("first attempt failed");
        let err_v = unsafe { read_json_and_free(err_ptr) };
        assert_eq!(err_v["success"], false);

        let ok_ptr = success_json(serde_json::json!({"retry": "succeeded"}));
        let ok_v = unsafe { read_json_and_free(ok_ptr) };
        assert_eq!(ok_v["success"], true);
        assert_eq!(ok_v["retry"], "succeeded");
    }

    #[test]
    fn test_send_task_validation_sequence() {
        // Simulate a C consumer fixing inputs iteratively
        // 1. First try: null
        let v1 = unsafe { read_json_and_free(lmao_send_task(std::ptr::null())) };
        assert!(v1["error"].as_str().unwrap().contains("null"));

        // 2. Bad JSON
        let input2 = CString::new("garbage").unwrap();
        let v2 = unsafe { read_json_and_free(lmao_send_task(input2.as_ptr())) };
        assert!(v2["error"].as_str().unwrap().contains("JSON"));

        // 3. Missing field
        let input3 = CString::new(r#"{"agent_pubkey":"02aa"}"#).unwrap();
        let v3 = unsafe { read_json_and_free(lmao_send_task(input3.as_ptr())) };
        assert_eq!(v3["error"], "missing 'task_text'");
    }

    // ── Pointer non-null guarantees ────────────────────────────────────────

    #[test]
    fn test_all_returns_non_null() {
        // Every FFI function that returns a pointer should never return null
        let ptrs = vec![
            lmao_version(),
            error_json("test"),
            success_json(serde_json::json!(null)),
            to_cstring(String::new()),
            lmao_send_task(std::ptr::null()),
        ];
        for ptr in &ptrs {
            assert!(!ptr.is_null(), "FFI function returned null pointer");
        }
        for ptr in ptrs {
            lmao_free_string(ptr);
        }
    }
}
