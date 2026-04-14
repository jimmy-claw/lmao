/* LMAO FFI — C interface for A2A over Waku */

#ifndef LMAO_FFI_H
#define LMAO_FFI_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Discover agents on the Waku network.
 *
 * args_json: { "timeout_ms": 5000 }  (optional, default 5000)
 *
 * Returns: { "success": true, "agents": [ { "name": "...", ... }, ... ] }
 */
char *lmao_discover_agents(const char *args_json);

/**
 * Send a text task to another agent.
 *
 * args_json: { "agent_pubkey": "02...", "task_text": "Hello" }
 *
 * Returns: { "success": true, "task_id": "...", "acked": true/false }
 */
char *lmao_send_task(const char *args_json);

/**
 * Get this agent's card as JSON.
 *
 * Returns: { "success": true, "card": { "name": "...", ... } }
 */
char *lmao_get_agent_card(void);

/**
 * Free a string returned by any lmao_* function.
 */
void lmao_free_string(char *s);

/**
 * Returns the version string of this FFI library.
 */
char *lmao_version(void);

/*
 * Delivery transport now uses logos_core_call_plugin_method_async directly
 * from Rust. The old callback injection (lmao_qtro_set_callbacks) and
 * callback typedefs are removed.
 *
 * lmao_qtro_on_message() is still available from the transport crate for
 * backward compatibility with C++ hosts that forward messageReceived events.
 */

#endif  /* LMAO_FFI_H */
