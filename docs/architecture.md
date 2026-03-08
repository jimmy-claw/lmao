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
