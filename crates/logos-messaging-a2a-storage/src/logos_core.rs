//! Raw FFI bindings to `liblogos_core`.
//!
//! These functions are resolved at link time against `liblogos_core.so`.
//! Set the `LOGOS_CORE_LIB_DIR` environment variable to the directory
//! containing the library when building with `--features logos-core`.

use std::os::raw::{c_char, c_int, c_void};

extern "C" {
    /// Invoke a plugin method asynchronously via the Logos Core C API.
    ///
    /// Calls `method_name` on `plugin_name` with the given JSON-encoded
    /// parameters. When the method completes, `callback` is invoked with
    /// the result code, an optional message string, and the opaque
    /// `user_data` pointer passed in by the caller.
    pub fn logos_core_call_plugin_method_async(
        plugin_name: *const c_char,
        method_name: *const c_char,
        params_json: *const c_char,
        callback: extern "C" fn(result: c_int, message: *const c_char, user_data: *mut c_void),
        user_data: *mut c_void,
    );

    /// Register a persistent event listener on a Logos Core plugin.
    ///
    /// Subscribes to `event_name` events emitted by `plugin_name`. Each
    /// time the event fires, `callback` is invoked with the result code,
    /// an optional message string, and the opaque `user_data` pointer.
    /// Unlike [`logos_core_call_plugin_method_async`], the callback may
    /// be invoked multiple times (once per event occurrence).
    pub fn logos_core_register_event_listener(
        plugin_name: *const c_char,
        event_name: *const c_char,
        callback: extern "C" fn(result: c_int, message: *const c_char, user_data: *mut c_void),
        user_data: *mut c_void,
    );
}
