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
 * Get a snapshot of operational metrics as JSON.
 *
 * Returns: { "success": true, "metrics": { "tasks_sent": 0, ... } }
 */
char *lmao_get_metrics(void);

/**
 * Get node info: identity, topics, encryption status, peer/session counts.
 *
 * Returns: { "success": true, "info": { "name": "...", ... } }
 */
char *lmao_get_node_info(void);

/**
 * Get live peers as JSON array.
 *
 * Returns: { "success": true, "peers": [ ... ] }
 */
char *lmao_get_peers(void);

/**
 * Get active sessions as JSON array.
 *
 * Returns: { "success": true, "sessions": [ ... ] }
 */
char *lmao_get_sessions(void);

/**
 * Free a string returned by any lmao_* function.
 */
void lmao_free_string(char *s);

/**
 * Returns the version string of this FFI library.
 */
char *lmao_version(void);

#endif  /* LMAO_FFI_H */
