/**
 * logos_core_stub.c — Stub liblogos_core for LMAO e2e demo.
 *
 * Provides in-process implementations of delivery_module and storage_module
 * that simulate Logos Core IPC without requiring the real SDK or plugins.
 *
 * delivery_module: loopback pub/sub (send → messageReceived for all listeners)
 * storage_module:  in-memory CID-addressed blob store
 *
 * NOTE: The transport crate uses 2-arg callbacks (const char*, void*) while
 * the storage crate uses 3-arg callbacks (int, const char*, void*). The stub
 * dispatches based on plugin name.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <unistd.h>

/* ---------------------------------------------------------------------------
 * Callback types (matching Rust FFI declarations)
 * ---------------------------------------------------------------------------*/

/* Transport crate: logos_core.rs */


/* Storage crate: logos_core.rs */
typedef void (*AsyncCallback)(int result, const char *message, void *user_data);

/* ---------------------------------------------------------------------------
 * Global state
 * ---------------------------------------------------------------------------*/

static pthread_mutex_t g_mutex = PTHREAD_MUTEX_INITIALIZER;

/* Event listeners */
#define MAX_LISTENERS 64

typedef struct {
    char plugin[64];
    char event[64];
    int  is_storage;          /* 0 = transport (2-arg), 1 = storage (3-arg) */
    union {
        AsyncCallback transport;
        AsyncCallback   storage;
    } cb;
    void *user_data;
} EventListener;

static EventListener g_listeners[MAX_LISTENERS];
static int g_listener_count = 0;

/* In-memory blob store: each blob stores its base64 chunks individually */
#define MAX_BLOBS  256
#define MAX_CHUNKS 64

typedef struct {
    char  cid[128];
    char *chunks[MAX_CHUNKS];   /* each chunk is a heap-allocated base64 string */
    int   chunk_count;
} BlobSlot;

static BlobSlot g_blobs[MAX_BLOBS];
static int g_blob_count = 0;
static int g_cid_counter = 0;

/* Upload sessions */
#define MAX_SESSIONS 32

typedef struct {
    int   id;
    int   active;
    char *chunks[MAX_CHUNKS];
    int   chunk_count;
} UploadSession;

static UploadSession g_sessions[MAX_SESSIONS];
static int g_session_count = 0;

/* Delivery send buffer size */
#define MAX_SEND_BUF (4 * 1024 * 1024)

/* ---------------------------------------------------------------------------
 * JSON helpers (minimal — just enough for the params format)
 *
 * Params format:
 *   [{"name":"key","value":"val","type":"string"}, ...]
 * ---------------------------------------------------------------------------*/

/**
 * Extract the "value" for a given "name" from the params JSON array.
 * Allocates and returns a new string (caller must free), or NULL if not found.
 */
static char *extract_value_alloc(const char *json, const char *name)
{
    if (!json || !name) return NULL;

    /* Search for "name":"<name>" */
    char needle[256];
    snprintf(needle, sizeof(needle), "\"name\":\"%s\"", name);

    const char *pos = strstr(json, needle);
    if (!pos) return NULL;

    /* Find "value":" after the name match */
    const char *vstart = strstr(pos, "\"value\":\"");
    if (!vstart) return NULL;
    vstart += 9;  /* skip past "value":" */

    /* Find the end of the value (closing quote) */
    const char *vend = vstart;
    while (*vend && !(*vend == '"' && *(vend - 1) != '\\'))
        vend++;

    size_t len = (size_t)(vend - vstart);
    char *result = (char *)malloc(len + 1);
    if (!result) return NULL;
    memcpy(result, vstart, len);
    result[len] = '\0';
    return result;
}

/* ---------------------------------------------------------------------------
 * Event firing helpers
 * ---------------------------------------------------------------------------*/

static void fire_transport_events(const char *plugin, const char *event,
                                  const char *payload_json)
{
    for (int i = 0; i < g_listener_count; i++) {
        EventListener *l = &g_listeners[i];
        if (!l->is_storage &&
            strcmp(l->plugin, plugin) == 0 &&
            strcmp(l->event, event) == 0)
        {
            l->cb.transport(1, payload_json, l->user_data);
        }
    }
}

static void fire_storage_events(const char *plugin, const char *event,
                                int result, const char *message)
{
    for (int i = 0; i < g_listener_count; i++) {
        EventListener *l = &g_listeners[i];
        if (l->is_storage &&
            strcmp(l->plugin, plugin) == 0 &&
            strcmp(l->event, event) == 0)
        {
            l->cb.storage(result, message, l->user_data);
        }
    }
}

/* ---------------------------------------------------------------------------
 * delivery_module handler
 * ---------------------------------------------------------------------------*/

static void handle_delivery(const char *method, const char *params,
                            AsyncCallback cb, void *ud)
{
    if (strcmp(method, "createNode") == 0 ||
        strcmp(method, "start") == 0 ||
        strcmp(method, "subscribe") == 0 ||
        strcmp(method, "unsubscribe") == 0)
    {
        cb("true", ud);
        return;
    }

    if (strcmp(method, "send") == 0) {
        char *topic   = extract_value_alloc(params, "contentTopic");
        char *payload = extract_value_alloc(params, "payload");

        if (topic && payload) {
            /* Build messageReceived JSON event */
            size_t event_len = strlen(topic) + strlen(payload) + 128;
            char *event_json = (char *)malloc(event_len);
            if (event_json) {
                snprintf(event_json, event_len,
                         "{\"contentTopic\":\"%s\",\"payload\":\"%s\"}",
                         topic, payload);

                pthread_mutex_lock(&g_mutex);
                fire_transport_events("delivery_module", "messageReceived",
                                      event_json);
                pthread_mutex_unlock(&g_mutex);

                free(event_json);
            }
        }

        free(topic);
        free(payload);
        cb("ok", ud);
        return;
    }

    /* Unknown method */
    cb("error: unknown method", ud);
}

/* ---------------------------------------------------------------------------
 * Delayed event helper — fires a storage event on a background thread.
 *
 * The download code in LogosCoreStorageBackend uses tokio::select! over
 * chunk and done channels. If both are ready simultaneously, select picks
 * randomly and may choose "done" first, losing chunks. By deferring the
 * done event to a background thread, we ensure the tokio select loop has
 * time to drain chunk events before the done signal arrives — matching
 * real Logos Core's async IPC timing.
 * ---------------------------------------------------------------------------*/

typedef struct {
    char plugin[64];
    char event[64];
    int  result;
    char message[256];
    int  delay_us;
} DelayedEventArgs;

static void *delayed_event_thread(void *arg)
{
    DelayedEventArgs *dea = (DelayedEventArgs *)arg;
    usleep(dea->delay_us);

    pthread_mutex_lock(&g_mutex);
    fire_storage_events(dea->plugin, dea->event, dea->result, dea->message);
    pthread_mutex_unlock(&g_mutex);

    free(dea);
    return NULL;
}

static void schedule_storage_event(const char *plugin, const char *event,
                                   int result, const char *message,
                                   int delay_us)
{
    DelayedEventArgs *dea = (DelayedEventArgs *)calloc(1, sizeof(*dea));
    strncpy(dea->plugin, plugin, sizeof(dea->plugin) - 1);
    strncpy(dea->event, event, sizeof(dea->event) - 1);
    dea->result = result;
    strncpy(dea->message, message, sizeof(dea->message) - 1);
    dea->delay_us = delay_us;

    pthread_t tid;
    pthread_create(&tid, NULL, delayed_event_thread, dea);
    pthread_detach(tid);
}

/* ---------------------------------------------------------------------------
 * storage_module handler
 * ---------------------------------------------------------------------------*/

static void handle_storage(const char *method, const char *params,
                           AsyncCallback cb, void *ud)
{
    if (strcmp(method, "uploadInit") == 0) {
        pthread_mutex_lock(&g_mutex);
        int idx = g_session_count++;
        g_sessions[idx].id = idx;
        g_sessions[idx].active = 1;
        g_sessions[idx].chunk_count = 0;
        memset(g_sessions[idx].chunks, 0, sizeof(g_sessions[idx].chunks));
        pthread_mutex_unlock(&g_mutex);

        char sid[32];
        snprintf(sid, sizeof(sid), "%d", idx);
        cb(0, sid, ud);
        return;
    }

    if (strcmp(method, "uploadChunk") == 0) {
        char *sid_str = extract_value_alloc(params, "sessionId");
        char *chunk   = extract_value_alloc(params, "chunk");
        int sid = sid_str ? atoi(sid_str) : -1;

        pthread_mutex_lock(&g_mutex);
        if (sid >= 0 && sid < g_session_count && g_sessions[sid].active) {
            int ci = g_sessions[sid].chunk_count;
            if (ci < MAX_CHUNKS && chunk) {
                g_sessions[sid].chunks[ci] = strdup(chunk);
                g_sessions[sid].chunk_count++;
            }
        }
        pthread_mutex_unlock(&g_mutex);

        free(sid_str);
        free(chunk);
        cb(0, "ok", ud);
        return;
    }

    if (strcmp(method, "uploadFinalize") == 0) {
        char *sid_str = extract_value_alloc(params, "sessionId");
        int sid = sid_str ? atoi(sid_str) : -1;
        free(sid_str);

        char cid[128];
        pthread_mutex_lock(&g_mutex);
        if (sid >= 0 && sid < g_session_count && g_sessions[sid].active) {
            /* Generate a CID */
            snprintf(cid, sizeof(cid), "zStub%04d", g_cid_counter++);

            /* Store the blob: move chunks from session to blob */
            if (g_blob_count < MAX_BLOBS) {
                BlobSlot *slot = &g_blobs[g_blob_count++];
                strncpy(slot->cid, cid, sizeof(slot->cid) - 1);
                slot->chunk_count = g_sessions[sid].chunk_count;
                for (int i = 0; i < slot->chunk_count; i++) {
                    slot->chunks[i] = g_sessions[sid].chunks[i];
                    g_sessions[sid].chunks[i] = NULL;
                }
            }
            g_sessions[sid].active = 0;
            g_sessions[sid].chunk_count = 0;
        } else {
            snprintf(cid, sizeof(cid), "error");
        }

        /* Fire storageUploadDone event with the CID */
        fire_storage_events("storage_module", "storageUploadDone", 0, cid);
        pthread_mutex_unlock(&g_mutex);

        cb(0, "ok", ud);
        return;
    }

    if (strcmp(method, "downloadChunks") == 0) {
        char *cid = extract_value_alloc(params, "cid");

        pthread_mutex_lock(&g_mutex);
        BlobSlot *found = NULL;
        for (int i = 0; i < g_blob_count; i++) {
            if (cid && strcmp(g_blobs[i].cid, cid) == 0) {
                found = &g_blobs[i];
                break;
            }
        }

        if (found) {
            /* Fire chunk events synchronously — they buffer in the Rust
               UnboundedReceiver immediately. */
            for (int i = 0; i < found->chunk_count; i++) {
                fire_storage_events("storage_module", "storageDownloadChunk",
                                    0, found->chunks[i]);
            }
        }
        int blob_found = (found != NULL);
        pthread_mutex_unlock(&g_mutex);
        free(cid);

        /* Schedule the done event on a background thread so the Rust
           tokio::select! loop drains all chunk events first. */
        if (blob_found) {
            schedule_storage_event("storage_module", "storageDownloadDone",
                                   0, "ok", 20000 /* 20 ms */);
        } else {
            schedule_storage_event("storage_module", "storageDownloadDone",
                                   1, "CID not found", 20000);
        }

        /* Return from the method call so call_method().await resolves. */
        cb(0, "ok", ud);
        return;
    }

    /* Unknown method */
    cb(1, "unknown storage method", ud);
}

/* ---------------------------------------------------------------------------
 * Public API: logos_core_*
 * ---------------------------------------------------------------------------*/

void logos_core_init(void)
{
    fprintf(stderr, "[stub] logos_core_init()\n");
}

void logos_core_set_mode(int mode)
{
    fprintf(stderr, "[stub] logos_core_set_mode(%d)\n", mode);
}

void logos_core_cleanup(void)
{
    fprintf(stderr, "[stub] logos_core_cleanup()\n");
    /* Free allocated blobs */
    for (int i = 0; i < g_blob_count; i++) {
        for (int j = 0; j < g_blobs[i].chunk_count; j++) {
            free(g_blobs[i].chunks[j]);
            g_blobs[i].chunks[j] = NULL;
        }
    }
}

void logos_core_load_plugin(const char *name)
{
    fprintf(stderr, "[stub] logos_core_load_plugin(\"%s\")\n", name);
}

void logos_core_add_plugins_dir(const char *path)
{
    fprintf(stderr, "[stub] logos_core_add_plugins_dir(\"%s\")\n", path);
}

void logos_core_register_event_listener(
    const char *plugin_name,
    const char *event_name,
    void       *callback,
    void       *user_data)
{
    fprintf(stderr, "[stub] logos_core_register_event_listener(\"%s\", \"%s\")\n",
            plugin_name, event_name);

    pthread_mutex_lock(&g_mutex);
    if (g_listener_count < MAX_LISTENERS) {
        EventListener *l = &g_listeners[g_listener_count++];
        strncpy(l->plugin, plugin_name, sizeof(l->plugin) - 1);
        strncpy(l->event, event_name, sizeof(l->event) - 1);

        if (strcmp(plugin_name, "delivery_module") == 0) {
            l->is_storage = 0;
            l->cb.transport = (AsyncCallback)callback;
        } else {
            l->is_storage = 1;
            l->cb.storage = (AsyncCallback)callback;
        }
        l->user_data = user_data;
    }
    pthread_mutex_unlock(&g_mutex);
}

void logos_core_call_plugin_method_async(
    const char *plugin_name,
    const char *method_name,
    const char *params_json,
    void       *callback,
    void       *user_data)
{
    fprintf(stderr, "[stub] %s.%s()\n", plugin_name, method_name);

    if (strcmp(plugin_name, "delivery_module") == 0) {
        handle_delivery(method_name, params_json,
                        (AsyncCallback)callback, user_data);
    } else if (strcmp(plugin_name, "storage_module") == 0) {
        handle_storage(method_name, params_json,
                       (AsyncCallback)callback, user_data);
    } else {
        fprintf(stderr, "[stub] unknown plugin: %s\n", plugin_name);
    }
}

/* Additional symbols that may be referenced */
void logos_core_exec(void) {}
const char *logos_core_get_loaded_plugins(void) { return "[]"; }
const char *logos_core_get_known_plugins(void) { return "[]"; }
void logos_core_load_static_plugins(void) {}
void logos_core_get_module_stats(void) {}
void logos_core_async_operation(void) {}
void logos_core_load_plugin_async(void) {}
void logos_core_process_events(void) {}
void logos_core_process_plugin(void) {}
void logos_core_register_plugin_by_name(void) {}
void logos_core_register_plugin_instance(void) {}
const char *logos_core_get_token(void) { return "stub-token"; }
void logos_core_load_plugin_with_dependencies(void) {}
