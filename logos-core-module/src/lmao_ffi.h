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
 * Build a task envelope without sending it — for use with external transports (QtRO).
 *
 * args_json: { "agent_pubkey": "02...", "task_text": "Hello" }
 *
 * Returns: { "success": true, "task_id": "...", "topic": "/lmao/1/task/...",
 *            "payload_b64": "base64-encoded A2A envelope" }
 *
 * The caller (e.g. C++ DeliveryTransport) can send the payload to the topic
 * via QtRO inter-module call to delivery_module, bypassing the Rust transport (issue #143).
 */
char *lmao_build_task_envelope(const char *args_json);

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
 * Get agent info: identity, topics, and encryption status.
 *
 * Returns: { "success": true, "public_key": "02...", "task_topic": "...",
 *            "discovery_topic": "...", "presence_topic": "...", "encryption": false }
 */
char *lmao_get_info(void);

/**
 * Get operational metrics counters.
 *
 * Returns: { "success": true, "tasks_sent": 0, "tasks_received": 0, ... }
 */
char *lmao_get_metrics(void);

/**
 * Returns the version string of this FFI library.
 */
char *lmao_version(void);

#endif  /* LMAO_FFI_H */
