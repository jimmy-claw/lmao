#ifndef WAKU_A2A_FFI_H
#define WAKU_A2A_FFI_H

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Free a string returned by this library.
void waku_a2a_free_string(char *s);

/// Initialize a node. Returns 0 on success.
int waku_a2a_init(const char *name, const char *description,
                  const char *nwaku_url, bool encrypted);

/// Get this node's public key (hex). Caller must free.
char *waku_a2a_pubkey(void);

/// Get agent card as JSON. Caller must free.
char *waku_a2a_agent_card_json(void);

/// Announce on discovery topic. Returns 0 on success.
int waku_a2a_announce(void);

/// Discover agents. Returns JSON array. Caller must free.
char *waku_a2a_discover(void);

/// Send text to agent. Returns 0 on success.
int waku_a2a_send_text(const char *to_pubkey, const char *text);

/// Poll incoming tasks. Returns JSON array. Caller must free.
char *waku_a2a_poll_tasks(void);

/// Respond to a task. Returns 0 on success.
int waku_a2a_respond(const char *task_json, const char *result_text);

/// Shutdown the node.
void waku_a2a_shutdown(void);

#ifdef __cplusplus
}
#endif

#endif /* WAKU_A2A_FFI_H */
