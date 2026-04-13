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

/* ── QtRO Delivery Transport Bridge ─────────────────────────────────────── */

/**
 * Callback for publishing a message via QtRO delivery_module.
 * Returns 0 on success, non-zero on error.
 */
typedef int (*LmaoPublishFn)(const char *topic, const char *payload_b64, void *user_data);

/**
 * Callback for subscribing to a topic via QtRO delivery_module.
 * Returns 0 on success, non-zero on error.
 */
typedef int (*LmaoSubscribeFn)(const char *topic, void *user_data);

/**
 * Callback for unsubscribing from a topic via QtRO delivery_module.
 * Returns 0 on success, non-zero on error.
 */
typedef int (*LmaoUnsubscribeFn)(const char *topic, void *user_data);

/**
 * Register QtRO delivery callbacks. Called once during module init.
 * Returns 1 on success, 0 if already registered.
 */
int lmao_qtro_set_callbacks(
    LmaoPublishFn publish_fn,
    LmaoSubscribeFn subscribe_fn,
    LmaoUnsubscribeFn unsubscribe_fn,
    void *user_data
);

/**
 * Called from C++ when delivery_module emits a message on a subscribed topic.
 * topic and payload_b64 must be valid null-terminated UTF-8 strings.
 */
void lmao_qtro_on_message(const char *topic, const char *payload_b64);

#endif  /* LMAO_FFI_H */
