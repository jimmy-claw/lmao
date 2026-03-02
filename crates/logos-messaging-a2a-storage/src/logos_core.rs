//! Raw FFI bindings to `liblogos_core`.
//!
//! These functions are resolved at link time against `liblogos_core.so`.
//! Set the `LOGOS_CORE_LIB_DIR` environment variable to the directory
//! containing the library when building with `--features logos-core`.

use std::os::raw::{c_char, c_int, c_void};

extern "C" {
    pub fn logos_core_call_plugin_method_async(
        plugin_name: *const c_char,
        method_name: *const c_char,
        params_json: *const c_char,
        callback: extern "C" fn(result: c_int, message: *const c_char, user_data: *mut c_void),
        user_data: *mut c_void,
    );

    pub fn logos_core_register_event_listener(
        plugin_name: *const c_char,
        event_name: *const c_char,
        callback: extern "C" fn(result: c_int, message: *const c_char, user_data: *mut c_void),
        user_data: *mut c_void,
    );
}
