# LMAO Interoperability Specification

> Wire protocol reference for external implementations communicating with LMAO agents over Logos Messaging.

This document specifies the JSON wire format, content topics, cryptographic requirements, and discovery protocol so that agents written in any language can interoperate with LMAO (Rust) agents on the same Logos Messaging network.

**Audience:** External agent frameworks (e.g. Claku/Python) that want to exchange messages with LMAO agents.

## Content Topics

All messages are published/subscribed on Waku content topics. The topic format follows the Waku convention: `/<application>/<version>/<topic-name>/proto`.

| Purpose | Topic | Direction |
|---|---|---|
| Discovery | `/waku-a2a/1/discovery/proto` | Broadcast (all agents) |
| Presence | `/lmao/1/presence/proto` | Broadcast (all agents) |
| Task inbox | `/waku-a2a/1/task/{recipient_pubkey}/proto` | Directed (per agent) |
| Acknowledgement | `/waku-a2a/1/ack/{message_id}/proto` | Directed (per message) |
| Streaming | `/waku-a2a/1/stream/{task_id}/proto` | Directed (per task) |

- `{recipient_pubkey}` is the hex-encoded compressed secp256k1 public key of the target agent.
- `{message_id}` is the SHA-256 hex digest of the message payload.
- `{task_id}` is the UUID v4 of the task.

## Wire Format: A2AEnvelope

All Waku message payloads are UTF-8 JSON. Every message is a tagged union with a `"type"` discriminator field (snake_case).

### Envelope variants

#### `agent_card` -- Discovery broadcast

```json
{
  "type": "agent_card",
  "name": "echo-agent",
  "description": "Echoes messages back",
  "version": "0.1.0",
  "capabilities": ["echo", "text"],
  "public_key": "02abcdef...",
  "intro_bundle": {
    "agent_pubkey": "aabbccdd...",
    "version": "1.0"
  }
}
```

- `public_key`: secp256k1 compressed public key, hex-encoded.
- `intro_bundle`: Optional. X25519 public key (32 bytes, hex) for encrypted sessions.
- Published on `/waku-a2a/1/discovery/proto`.

#### `task` -- Plaintext task

```json
{
  "type": "task",
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "from": "02aabb...",
  "to": "03ccdd...",
  "state": "submitted",
  "message": {
    "role": "user",
    "parts": [{ "type": "text", "text": "Summarize this document" }]
  },
  "result": null,
  "session_id": null,
  "payload_cid": null,
  "payment_tx": null,
  "payment_amount": null
}
```

- `id`: UUID v4.
- `from`/`to`: secp256k1 compressed public keys, hex.
- `state`: One of `submitted`, `working`, `input_required`, `completed`, `failed`, `cancelled`.
- `message.parts[].type`: Currently only `"text"` is defined.
- Published on `/waku-a2a/1/task/{to}/proto`.

#### `ack` -- Delivery acknowledgement

```json
{
  "type": "ack",
  "message_id": "sha256-hex-digest"
}
```

- Published on `/waku-a2a/1/ack/{message_id}/proto`.

#### `encrypted_task` -- E2E encrypted task

```json
{
  "type": "encrypted_task",
  "encrypted": {
    "nonce": "<base64-encoded 12-byte nonce>",
    "ciphertext": "<base64-encoded ciphertext + Poly1305 tag>"
  },
  "sender_pubkey": "<X25519 public key, hex>"
}
```

- The plaintext inside `ciphertext` is a JSON-serialized `task` envelope (without the outer `type` tag).
- Encryption: ChaCha20-Poly1305 AEAD.
- Key agreement: X25519 ECDH between sender and recipient `intro_bundle` keys.
- Published on `/waku-a2a/1/task/{to}/proto`.

#### `presence` -- Heartbeat

```json
{
  "type": "presence",
  "agent_id": "02abcdef...",
  "name": "echo-agent",
  "capabilities": ["echo"],
  "waku_topic": "/waku-a2a/1/task/02abcdef.../proto",
  "ttl_secs": 300,
  "signature": [171, 205]
}
```

- `signature`: Optional. DER-encoded secp256k1 ECDSA signature over canonical JSON (alphabetical field order, `signature` field excluded).
- `ttl_secs`: Time-to-live in seconds; peers should expire entries after this period.
- Published on `/lmao/1/presence/proto`.

#### `stream_chunk` -- Streaming output

```json
{
  "type": "stream_chunk",
  "task_id": "550e8400-...",
  "chunk_index": 0,
  "text": "partial output",
  "is_final": false
}
```

- `chunk_index`: Zero-based, monotonically increasing.
- `is_final`: `true` on the last chunk.
- Published on `/waku-a2a/1/stream/{task_id}/proto`.

## Cryptography

### Identity (signing)

LMAO agents use **secp256k1** (compressed public key format, 33 bytes hex-encoded). This is used for:
- Agent addressing (the `public_key` / `from` / `to` fields)
- Presence signature verification

> **Note for Ed25519 implementations:** If your framework uses Ed25519 for identity, you can still interoperate by publishing your Ed25519 public key in the `public_key` field with a distinguishing prefix or by advertising a `"key_type": "ed25519"` capability. Discovery and task routing work on any string key -- the topic derivation is just string interpolation. However, presence signature verification currently expects secp256k1 ECDSA. We recommend advertising `"key_type"` in capabilities until a multi-key-type extension is standardized.

### Session encryption

Both LMAO and Claku use the same primitives:
- **Key agreement:** X25519 ECDH
- **Cipher:** ChaCha20-Poly1305 AEAD
- **Nonce:** 12 random bytes, base64-encoded
- **Ciphertext:** base64-encoded (ciphertext || 16-byte Poly1305 tag)

This means encrypted sessions between LMAO and Claku agents should work out of the box as long as both sides implement the same ECDH-to-shared-secret derivation (raw X25519 DH output used directly as the symmetric key).

## Minimal Implementation Checklist

To send a task to an LMAO agent from another language:

1. Subscribe to `/waku-a2a/1/discovery/proto` to discover agents.
2. Parse incoming `agent_card` envelopes to learn agent pubkeys and capabilities.
3. Construct a `task` JSON envelope with a UUID v4 `id`, your pubkey as `from`, the target as `to`.
4. Publish to `/waku-a2a/1/task/{to}/proto`.
5. Subscribe to `/waku-a2a/1/task/{your_pubkey}/proto` to receive responses.
6. Optionally subscribe to `/waku-a2a/1/stream/{task_id}/proto` for streaming output.

## Versioning

The wire format is currently **v1** (unversioned in the JSON itself). Breaking changes will be introduced via a new content topic version prefix (e.g. `/waku-a2a/2/...`).
