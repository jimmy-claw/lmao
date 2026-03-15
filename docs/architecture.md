# logos-messaging-a2a Architecture

## Full Stack Diagram

```
┌──────────────────────────────────────────────────────────────────────┐
│                        Application Layer                             │
│                                                                      │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐            │
│  │  lmao-cli    │   │  echo_agent  │   │  ping_pong   │            │
│  │  (CLI binary) │   │  (example)   │   │  (example)   │            │
│  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘            │
│         │                  │                   │                     │
│  ┌──────┴──────┐    ┌──────┴──────────────────┴──────┐             │
│  │  lmao-mcp   │    │         lmao-ffi / ffi         │             │
│  │  (MCP bridge)│    │  (C/Swift/Kotlin bindings)     │             │
│  └──────┬───────┘   └──────┬─────────────────────────┘             │
│         └──────────────────┼───────────────────────────              │
│                            │                                         │
├────────────────────────────┼─────────────────────────────────────────┤
│                     Node Layer                                       │
│                                                                      │
│  ┌─────────────────────────┴──────────────────────────────┐         │
│  │                 WakuA2ANode<T>                          │         │
│  │                                                         │         │
│  │  • announce()     — broadcast AgentCard                 │         │
│  │  • discover()     — find agents on network              │         │
│  │  • send_task()    — send task with SDS reliability      │         │
│  │  • poll_tasks()   — receive incoming tasks              │         │
│  │  • respond()      — reply to a task                     │         │
│  │  • presence       — PeerMap with heartbeat broadcasts   │         │
│  │  • delegate_task()— forward subtasks to capable peers  │         │
│  │                                                         │         │
│  │  Identity: secp256k1 keypair                            │         │
│  │  Integrates: crypto, execution, storage, transport      │         │
│  └────────┬──────────┬──────────┬──────────┬──────────────┘         │
│           │          │          │          │                          │
├───────────┼──────────┼──────────┼──────────┼─────────────────────────┤
│           │          │          │          │                          │
│  ┌────────┴───┐ ┌────┴─────┐ ┌─┴────────┐ │                        │
│  │   Crypto   │ │Execution │ │  Storage  │ │                        │
│  │            │ │          │ │           │ │                        │
│  │ X25519 DH  │ │ Status   │ │ Codex    │ │                        │
│  │ ChaCha20   │ │ Network  │ │ REST API │ │                        │
│  │ Poly1305   │ │ (EVM)    │ │          │ │                        │
│  │            │ │ LEZ stub │ │ LogosCore│ │                        │
│  │ IntroBundle│ │          │ │ backend  │ │                        │
│  └────────────┘ └──────────┘ └──────────┘ │                        │
│                                            │                         │
├────────────────────────────────────────────┼─────────────────────────┤
│                  Reliability Layer (minimal-SDS)                      │
│                                                                      │
│  ┌─────────────────────────────────────────┴──────────────┐         │
│  │              SdsTransport<T: WakuTransport>             │         │
│  │                                                         │         │
│  │  • publish_reliable() — retransmit up to 3x             │         │
│  │  • send_ack()         — acknowledge receipt             │         │
│  │  • poll_dedup()       — deduplicate by message ID       │         │
│  │  • causal ordering    — lamport clocks + buffering      │         │
│  │  • bloom filter dedup — probabilistic duplicate detect  │         │
│  │  • batch ACK          — coalesce acknowledgements       │         │
│  │                                                         │         │
│  │  ACK timeout: 10s | Max retries: 3                      │         │
│  └─────────────────────────┬──────────────────────────────┘         │
│                            │                                         │
├────────────────────────────┼─────────────────────────────────────────┤
│                  Transport Layer (swappable)                          │
│                                                                      │
│  ┌─────────────────────────┴──────────────────────────────┐         │
│  │            trait WakuTransport                          │         │
│  │                                                         │         │
│  │  • publish(topic, payload)                              │         │
│  │  • subscribe(topic)                                     │         │
│  │  • poll(topic) -> Vec<Vec<u8>>                          │         │
│  │                                                         │         │
│  ├─────────────────────────────────────────────────────────┤         │
│  │                                                         │         │
│  │  NwakuRestTransport        LogosDeliveryTransport       │         │
│  │  (v0.1 — REST fallback)    (planned — issue #57)        │         │
│  │  http://localhost:8645     logos-delivery-rust-bindings  │         │
│  │                                                         │         │
│  └─────────────────────────┬──────────────────────────────┘         │
│                            │                                         │
├────────────────────────────┼─────────────────────────────────────────┤
│                     Waku Network                                     │
│                                                                      │
│  ┌─────────────────────────┴──────────────────────────────┐         │
│  │              Waku Relay (pub/sub)                        │         │
│  │                                                         │         │
│  │  Content Topics:                                        │         │
│  │  /lmao/1/discovery/proto        AgentCard broadcasts    │         │
│  │  /lmao/1/presence/proto         Presence heartbeats     │         │
│  │  /lmao/1/task/{pubkey}/proto    Task inbox per agent    │         │
│  │  /lmao/1/ack/{msg_id}/proto     SDS acknowledgements   │         │
│  │                                                         │         │
│  └─────────────────────────────────────────────────────────┘         │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────┐         │
│  │  nwaku node (relay, store, filter)                      │         │
│  │  OR embedded libwaku via logos-delivery-rust-bindings    │         │
│  └─────────────────────────────────────────────────────────┘         │
└──────────────────────────────────────────────────────────────────────┘
```

## Crate Dependency Graph

```
logos-messaging-a2a (workspace root)
│
├── logos-messaging-a2a-core           A2A protocol types (AgentCard, Task, etc.)
│   └── depends on: crypto
│
├── logos-messaging-a2a-crypto         X25519 ECDH + ChaCha20-Poly1305 encryption
│   └── no internal deps
│
├── logos-messaging-a2a-transport      Waku transport trait + SDS reliability layer
│   └── no internal deps
│
├── logos-messaging-a2a-storage        Storage backends (Codex REST, LogosCore)
│   └── no internal deps
│
├── logos-messaging-a2a-execution      On-chain execution (Status Network, LEZ stub)
│   └── depends on: core
│
├── logos-messaging-a2a-node           WakuA2ANode — main orchestrator
│   └── depends on: core, crypto, transport, storage, execution
│
├── logos-messaging-a2a-cli            CLI binary
│   └── depends on: core, crypto, transport, node
│
├── logos-messaging-a2a-mcp            MCP bridge (stdio server for Claude/Cursor)
│   └── depends on: core, transport, node
│
├── logos-messaging-a2a-ffi            C-ABI FFI bindings (UniFFI)
│   └── depends on: core, crypto, transport, node
│
└── lmao-ffi                           Thin C FFI wrapper
    └── depends on: core, transport, node
```

## A2A Types (logos-messaging-a2a-core)

```
AgentCard
├── name: String
├── description: String
├── version: String
├── capabilities: Vec<String>
├── public_key: String              (secp256k1 compressed hex)
└── intro_bundle: Option<IntroBundle>

Task
├── id: String                      (UUID v4)
├── from: String                    (sender pubkey)
├── to: String                      (recipient pubkey)
├── state: TaskState                (Submitted → Working → Completed/Failed)
├── message: Message
│   ├── role: String                ("user" or "agent")
│   └── parts: Vec<Part>
│       └── Part::Text { text }
└── result: Option<Message>         (agent's response)

A2AEnvelope (wire format)
├── AgentCard(AgentCard)
├── Task(Task)
├── Ack { message_id }
└── Presence(PresenceAnnounce)
```

## Crypto Layer

```
AgentIdentity (X25519)
├── generate()                      → random keypair
├── public_key_hex()                → hex-encoded pubkey
├── shared_key(their_pubkey)        → SessionKey via ECDH
└── from_hex(secret)                → reconstruct from secret

SessionKey (ChaCha20-Poly1305)
├── encrypt(plaintext)              → EncryptedPayload (nonce + ciphertext)
└── decrypt(payload)                → plaintext bytes

IntroBundle
├── agent_pubkey: String
└── version: String
```

## Presence Discovery

```
Agent A                    Waku Network                  Agent B
  │                            │                            │
  │── PresenceAnnounce ───────▶│ /lmao/1/presence/proto     │
  │   { pubkey, name,          │                            │
  │     capabilities,          │                            │
  │     timestamp }            │                            │
  │                            │◀── PresenceAnnounce ───────│
  │                            │                            │
  │   PeerMap tracks all       │                            │
  │   seen agents with TTL     │                            │
  │   (auto-expire stale)      │                            │
```

## Message Flow

```
Agent A                    Waku Network                  Agent B
  │                            │                            │
  │── announce(AgentCard) ────▶│ /lmao/1/discovery/proto    │
  │                            │◀── announce(AgentCard) ────│
  │                            │                            │
  │── discover() ─────────────▶│                            │
  │◀── [AgentCard B] ─────────│                            │
  │                            │                            │
  │── send_task(Task) ────────▶│ /lmao/1/task/{B}/proto     │
  │                            │──────── poll_tasks() ─────▶│
  │                            │                            │
  │   (SDS: wait for ACK)      │◀── send_ack(task.id) ─────│
  │◀── ACK on /ack/{id}/proto─│                            │
  │                            │                            │
  │                            │◀── respond(result) ───────│
  │◀── poll_tasks() ──────────│ /lmao/1/task/{A}/proto     │
  │                            │                            │
```

## x402 Payment Flow

```
Agent A (client)           Waku Network            Agent B (paywall)
  │                            │                            │
  │── send_task(request) ─────▶│───────────────────────────▶│
  │                            │                            │
  │◀── 402 PaymentRequired ───│◀───────────────────────────│
  │   { token_contract,       │                            │
  │     recipient, amount,    │                            │
  │     network }             │                            │
  │                            │                            │
  │── ERC-20 transfer ────────▶│  (on-chain via execution)  │
  │                            │                            │
  │── send_task(request        │                            │
  │   + payment_tx_hash) ────▶│───────────────────────────▶│
  │                            │                     verify │
  │                            │                   transfer │
  │◀── Task(Completed) ───────│◀───────────────────────────│
```

## Storage Offload Flow

```
Agent A                    Codex Node               Agent B
  │                            │                       │
  │  payload > 100KB           │                       │
  │── upload(data) ───────────▶│                       │
  │◀── CID ───────────────────│                       │
  │                            │                       │
  │── Task { storage_cid }    │                       │
  │   via Waku ───────────────┼──────────────────────▶│
  │                            │                       │
  │                            │◀── download(CID) ────│
  │                            │── data ──────────────▶│
```

## Message Retry with Exponential Backoff

P2P transports are inherently unreliable. The retry layer wraps the SDS
`send_reliable` path and replays failed sends with exponential backoff.

```
RetryConfig
├── max_attempts: u32          default 5
├── base_delay_ms: u64         default 1 000 ms
├── max_delay_ms: u64          default 60 000 ms
└── jitter: bool               default true
```

Delay for attempt `n` (0-indexed):

```
min(base_delay_ms × 2^n, max_delay_ms)  [+ random jitter]
```

```
                        RetryLayer<T: Transport>
                               │
           attempt 0           │
   send_reliable() ───────────▶│──── Ok ──▶ return
                               │
                          Err? │
                               ▼
                   sleep(base_delay_ms)
                               │
           attempt 1           │
   send_reliable() ───────────▶│──── Ok ──▶ return
                               │
                          Err? │
                               ▼
                  sleep(base_delay_ms × 2)
                               │
              ...              │
                               │
       attempt max_attempts-1  │
   send_reliable() ───────────▶│──── Ok ──▶ return
                               │
                          Err? │
                               ▼
                    return final error
```

Key design points:
- Only **transport errors** (`Err`) are retried. A successful send that is
  not ACKed (`Ok((_, false))`) is left to the SDS retransmission loop.
- Jitter adds a uniform random offset in `[0, base_delay] / 2` to avoid
  thundering-herd problems when many agents retry simultaneously.
- Enable via `WakuA2ANode::with_retry(RetryConfig { ... })`.

Implementation:
- `logos-messaging-a2a-core::RetryConfig` — configuration type.
- `logos-messaging-a2a-node::retry::RetryLayer` — the retry wrapper.

## Waku Presence Broadcasts

Agents periodically broadcast `PresenceAnnouncement` messages on the
well-known topic `/lmao/1/presence/proto`. Peers listen, build a live
`PeerMap`, and query it by capability.

```
PresenceAnnouncement
├── agent_id: String           secp256k1 compressed pubkey hex
├── name: String               human-readable agent name
├── capabilities: Vec<String>  e.g. ["summarize", "translate"]
├── waku_topic: String         where this agent receives tasks
├── ttl_secs: u64              validity window
└── signature: Option<Vec<u8>> secp256k1 over canonical JSON
```

### Signature Verification

Announcements are signed over a **canonical JSON** serialization (fixed
key order, `signature` field excluded) using the agent's secp256k1 key.
Verifiers decode `agent_id` to a public key and check the DER-encoded
signature, rejecting tampered or spoofed announcements.

### PeerMap

```
PeerMap (Mutex<HashMap<agent_id, PeerInfo>>)
│
├── update(announcement)       insert / refresh an entry
├── get(agent_id)              lookup, returns None if expired
├── find_by_capability(cap)    filter live peers by capability
├── all_live()                 all non-expired peers
└── evict_expired()            garbage-collect stale entries
```

Entries expire when `now - last_seen > ttl_secs`. A TTL of 0 means the
entry is always considered expired (useful for one-shot announcements).
Expired entries are lazily filtered on read and batch-removed via
`evict_expired()`.

### Combined Discovery

`WakuA2ANode::discover_all()` merges two discovery sources and
deduplicates by public key:

```
                  ┌───────────────────────┐
                  │   discover_all()       │
                  └───────┬───────────────┘
                          │
              ┌───────────┼───────────────┐
              ▼                           ▼
   poll_presence()              registry.list_agents()
   (Waku topic scan)           (persistent on-chain)
              │                           │
              └───────────┬───────────────┘
                          ▼
                 deduplicate by pubkey
                          │
                          ▼
                  Vec<AgentCard>
```

Implementation:
- `logos-messaging-a2a-core::PresenceAnnouncement` — wire type + signing.
- `logos-messaging-a2a-node::presence::{PeerInfo, PeerMap}` — live peer tracking.

## Task Delegation

An orchestrator agent decomposes a parent task into subtasks and forwards
each subtask to a peer chosen from the live `PeerMap`.

```
Orchestrator               PeerMap                  Worker A / Worker B
  │                           │                           │
  │── DelegationRequest ─────▶│                           │
  │   { parent_task_id,       │  strategy:                │
  │     subtask_text,         │  FirstAvailable           │
  │     strategy,             │  CapabilityMatch("code")  │
  │     timeout_secs }        │  BroadcastCollect         │
  │                           │                           │
  │   select peer(s) ◀───────│                           │
  │                           │                           │
  │── Task(subtask) ─────────┼──────────────────────────▶│
  │   via transport.publish   │                           │
  │                           │                           │
  │   poll_tasks() loop       │                           │
  │   (up to timeout_secs)    │                           │
  │                           │                           │
  │◀── DelegationResult ─────┼───────────────────────────│
  │   { success, result_text, │                           │
  │     agent_id, error }     │                           │
```

### Key types

```
DelegationStrategy (tagged enum)
├── FirstAvailable                pick any live peer
├── CapabilityMatch { capability } pick a peer with matching capability
└── BroadcastCollect              send to all, collect every response

DelegationRequest
├── parent_task_id: String
├── subtask_text: String
├── strategy: DelegationStrategy
└── timeout_secs: u64             (0 = default 30s)

DelegationResult
├── parent_task_id: String
├── subtask_id: String
├── agent_id: String              pubkey of the worker
├── result_text: Option<String>
├── success: bool
└── error: Option<String>
```

Implementation:
- `logos-messaging-a2a-core::delegation` — wire types.
- `logos-messaging-a2a-node::delegation` — `delegate_task()` and `delegate_broadcast()`.

## MCP Bridge Architecture

The MCP bridge (`logos-messaging-a2a-mcp`) exposes discovered Waku A2A
agents as MCP tools, letting MCP hosts like Claude Desktop or Cursor
interact with the decentralized agent fleet.

```
┌─────────────────┐   stdio    ┌──────────────────────────┐   Waku    ┌──────────────┐
│   MCP Host      │◀─────────▶│   LogosA2ABridge          │◀────────▶│  Agent Fleet  │
│  (Claude, etc.) │            │                          │           │  (Waku P2P)   │
│                 │            │  ┌────────────────────┐  │           │               │
│  discover_agents│───────────▶│  │ WakuA2ANode<T>     │  │           │  Agent A      │
│  send_to_agent  │            │  │  .discover()       │──┼──────────▶│  Agent B      │
│  list_cached    │            │  │  .send_text()      │  │           │  Agent C      │
│                 │            │  │  .poll_tasks()     │  │           │               │
│                 │◀───────────│  └────────────────────┘  │           │               │
│   tool results  │            │                          │           │               │
│                 │            │  AgentRegistry (cache)    │           │               │
└─────────────────┘            └──────────────────────────┘           └──────────────┘
```

### MCP Tools

| Tool                | Description                                             |
|---------------------|---------------------------------------------------------|
| `discover_agents`   | Poll the Waku discovery topic; cache and return agents  |
| `send_to_agent`     | Send a message to a named agent and poll for a response |
| `list_cached_agents`| Return cached agents without a network call             |

### Request Flow

```
Claude Desktop            MCP Bridge                  Waku Network
  │                          │                            │
  │── discover_agents ──────▶│                            │
  │                          │── node.discover() ────────▶│
  │                          │◀── Vec<AgentCard> ────────│
  │                          │   cache in AgentRegistry   │
  │◀── agent list ──────────│                            │
  │                          │                            │
  │── send_to_agent ────────▶│                            │
  │   { name, message }      │── lookup name in cache     │
  │                          │── node.send_text() ───────▶│
  │                          │                            │
  │                          │── poll loop (2s interval)  │
  │                          │── node.poll_tasks() ──────▶│
  │                          │◀── Task(Completed) ───────│
  │◀── agent response ─────│                            │
```

The bridge runs as a stdio server (`rmcp` transport-io) — no HTTP server
is involved. It is configured via CLI flags:

- `--waku-url` — nwaku REST API endpoint (default `http://localhost:8645`)
- `--timeout` — seconds to wait for agent responses (default `30`)
