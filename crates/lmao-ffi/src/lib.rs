//! lmao-ffi — C FFI wrapper for LMAO (A2A over Waku).
//!
//! All functions accept/return JSON strings (UTF-8, null-terminated).
//! Caller must free returned strings with lmao_free_string().

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

use tokio::runtime::Runtime;
use waku_a2a_node::WakuA2ANode;
use waku_a2a_transport::nwaku_rest::NwakuTransport;

/// Global tokio runtime for async operations.
fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("Failed to create tokio runtime"))
}

/// Global node instance (lazy-initialized on first call).
static NODE: OnceLock<WakuA2ANode<NwakuTransport>> = OnceLock::new();

fn get_or_init_node() -> &'static WakuA2ANode<NwakuTransport> {
    NODE.get_or_init(|| {
        let waku_url =
            std::env::var("WAKU_URL").unwrap_or_else(|_| "http://localhost:8645".to_string());
        let transport = NwakuTransport::new(&waku_url);
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

fn cstr_to_str(ptr: *const c_char) -> Result<&'static str, String> {
    if ptr.is_null() {
        return Err("null pointer".to_string());
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|e| format!("Invalid UTF-8: {}", e))
}

fn to_cstring(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

fn error_json(msg: &str) -> *mut c_char {
    to_cstring(format!(
        r#"{{"success":false,"error":"{}"}}"#,
        msg.replace('"', "\\\"")
    ))
}

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
    use waku_a2a_core::{A2AEnvelope, AgentCard, Message, Part, Task, TaskState};

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
    fn test_to_cstring_and_back() {
        let original = "test string 123";
        let ptr = to_cstring(original.to_string());
        let recovered = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(recovered, original);
        lmao_free_string(ptr);
    }

    #[test]
    fn test_error_json_format() {
        let ptr = error_json("something went wrong");
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "something went wrong");
        lmao_free_string(ptr);
    }

    #[test]
    fn test_error_json_escapes_quotes() {
        let ptr = error_json(r#"bad "input""#);
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().contains("bad"));
        lmao_free_string(ptr);
    }

    #[test]
    fn test_success_json_object() {
        let ptr = success_json(serde_json::json!({"key": "value"}));
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v["success"], true);
        assert_eq!(v["key"], "value");
        lmao_free_string(ptr);
    }

    #[test]
    fn test_success_json_non_object() {
        let ptr = success_json(serde_json::json!(42));
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v["success"], true);
        assert_eq!(v["data"], 42);
        lmao_free_string(ptr);
    }

    #[test]
    fn test_version() {
        let ptr = lmao_version();
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
        lmao_free_string(ptr);
    }

    #[test]
    fn test_cstr_to_str_null() {
        assert!(cstr_to_str(std::ptr::null()).is_err());
    }

    #[test]
    fn test_cstr_to_str_valid() {
        let c = CString::new("hello").unwrap();
        let result = cstr_to_str(c.as_ptr());
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn test_runtime_init_idempotent() {
        let rt1 = runtime();
        let rt2 = runtime();
        assert!(std::ptr::eq(rt1, rt2));
    }

    // ── JSON serialization tests for FFI types ──

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
    fn test_envelope_task_json() {
        let task = Task::new("02aa", "03bb", "test");
        let envelope = A2AEnvelope::Task(task);
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
}
