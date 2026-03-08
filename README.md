# LMAO — Logos Module for Agent Orchestration

> **LMAO** = **L**ogos **M**odule for **A**gent **O**rchestration
>
> Yes, the acronym is intentional. Building decentralized AI agent infrastructure is serious work — but it doesn't have to be humourless. LMAO implements Google's [A2A protocol](https://github.com/google/A2A) over [Logos Messaging](https://logos.co/messaging/) decentralized transport, bringing censorship-resistant, serverless agent-to-agent communication to the Logos stack.

## The Problem

Google's A2A protocol is great. But it assumes HTTP: stable endpoints, central registries, easy censorship. That's fine for web2. For a decentralized agent network running on Logos, it's a non-starter.

**LMAO** replaces HTTP with Logos Messaging — a decentralized pub/sub network — giving you full A2A semantics with:

| | HTTP/SSE | LMAO (Logos Messaging) |
|---|---|---|
| Discovery | Central registry | Content-addressed pub/sub topics |
| Endpoints | Stable IP required | Just a pubkey |
| Privacy | Traffic analysis easy | Optional E2E encryption |
| Censorship | Single point of failure | Decentralized relay |
| NAT | Needs port forwarding | Works behind NAT |

## Architecture

```
┌─────────────────────────────────────────────────────┐
│              Logos Messaging Network                 │
│                                                      │
│  /lmao/1/discovery/proto     ← AgentCard broadcasts │
│  /lmao/1/task/{pubkey}/proto ← Task inbox per agent │
│  /lmao/1/ack/{msg_id}/proto  ← SDS acknowledgements │
│  /lmao/1/presence/proto       ← Peer discovery        │
└──────────┬──────────────┬──────────────┬─────────────┘
           │              │              │
      ┌────▼────┐    ┌───▼────┐    ┌───▼────┐
      │ Agent A │    │ Agent B│    │ Agent C│
      │ (echo)  │    │ (code) │    │(search)│
      └─────────┘    └────────┘    └────────┘
           │
      ┌────▼──────────────────────┐
      │  MCP Bridge               │  ← Claude Desktop / Cursor
      │  logos-messaging-a2a-mcp  │     can talk to any agent
      └───────────────────────────┘
           │
      ┌────▼──────────────────────┐
      │  Logos Core               │  ← Qt UI plugin
      │  logos-messaging-a2a-ffi  │     for the Logos desktop app
      └───────────────────────────┘
```

## Getting Started

### Path 1: MCP Bridge (works today)

Expose Logos Messaging A2A agents as MCP tools in Claude Desktop, Cursor, or any MCP-compatible host.

**1. Build the bridge**

```bash
cargo build -p logos-messaging-a2a-mcp --release
```

**2. Start a nwaku node**

```bash
docker run -p 8645:8645 statusteam/nim-waku:v0.31.0 \
  --rest --rest-address=0.0.0.0
```

**3. Add to your MCP config**

Claude Desktop (`claude_desktop_config.json`) or Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "logos-agents": {
      "command": "./target/release/logos-messaging-a2a-mcp",
      "args": ["--waku-url", "http://localhost:8645"]
    }
  }
}
```

**4. Available tools**

| Tool | Description |
|------|-------------|
| `discover_agents` | Find agents advertising on the Logos Messaging network |
| `send_to_agent` | Send a message to an agent by name and get a response |
| `list_cached_agents` | List agents from the last discovery (no network call) |

### Path 2.5: Native Waku Transport (no Docker needed)

Run a Waku node in-process using libwaku FFI — no separate nwaku container required.

**Prerequisites:** Nim 2.x (`choosenim`), Rust stable.

```bash
# Build with native-waku feature
cargo build -p logos-messaging-a2a-transport --features native-waku

# Use in code:
use logos_messaging_a2a_transport::NativeWakuTransport;
use waku_bindings::WakuNodeConfig;

let transport = NativeWakuTransport::new(WakuNodeConfig {
    tcp_port: Some(60010),
    ..Default::default()
}).await?;
transport.connect("/ip4/x.x.x.x/tcp/60000/p2p/...").await?;
```

The `NativeWakuTransport` implements the same `Transport` trait — drop-in replacement for `NwakuRestTransport`.

### Path 2: Logos Core IComponent (future)

Once the `.lgx` plugin is ready (auto-built on `v*` tags):

```bash
# 1. Download from GitHub Releases
wget https://github.com/jimmy-claw/lmao/releases/latest/download/lmao.lgx

# 2. Install via Logos package manager
lgpm install lmao.lgx

# 3. Agent fleet panel appears in Logos App
```

### Quick Start: Two Agents Talking

No nwaku needed — this runs entirely in-memory:

```rust
use anyhow::Result;
use waku_a2a::{A2AEnvelope, InMemoryTransport, Task, Transport, WakuA2ANode};

#[tokio::main]
async fn main() -> Result<()> {
    let transport = InMemoryTransport::new();

    // Create two agents on the same in-memory network
    let alice = WakuA2ANode::new(
        "alice", "Greeting agent", vec!["text".into()], transport.clone(),
    );
    let bob = WakuA2ANode::new(
        "bob", "Echo agent", vec!["text".into()], transport.clone(),
    );

    // Broadcast agent cards on the discovery topic
    alice.announce().await?;
    bob.announce().await?;

    // Alice discovers Bob
    let agents = alice.discover().await?;
    println!("Alice found {} agent(s)", agents.len());

    // Alice sends a task to Bob
    let task = Task::new(alice.pubkey(), bob.pubkey(), "Hello from Alice!");
    let envelope = A2AEnvelope::Task(task.clone());
    let payload = serde_json::to_vec(&envelope)?;
    let topic = waku_a2a::topics::task_topic(bob.pubkey());
    bob.poll_tasks().await?; // ensure Bob is subscribed
    transport.publish(&topic, &payload).await?;

    // Bob receives and responds
    let tasks = bob.poll_tasks().await?;
    let msg = tasks[0].text().unwrap();
    println!("Bob received: {msg}");
    bob.respond(&tasks[0], &format!("Echo: {msg}")).await?;

    // Alice reads the response
    let responses = alice.poll_tasks().await?;
    println!("Alice got: {}", responses[0].result_text().unwrap());

    Ok(())
}
```

Output:

```
Alice found 1 agent(s)
Bob received: Hello from Alice!
Alice got: Echo: Hello from Alice!
```

## Quick Start

```bash
# Ping-pong demo — no nwaku needed, fully in-memory
cargo run --example ping_pong

# With encryption
cargo run --example ping_pong -- --encrypt

# MCP bridge (requires nwaku running on :8645)
cargo run -p logos-messaging-a2a-mcp -- --waku-url http://localhost:8645

# Logos Core native demo (delivery_module + storage_module IPC)
make demo-logos-core
```

## Logos Core Native Demo

End-to-end demo exercising `LogosCoreDeliveryTransport` and `LogosCoreStorageBackend`
through the Logos Core C IPC API — no REST APIs, no mocks of the LMAO layer.

### Prerequisites

- Rust toolchain
- C compiler (`cc`)
- **Optional:** Logos Core SDK (`liblogos_core.so`) with `delivery_module` and `storage_module` plugins

### Run with stub (default)

The demo auto-compiles a stub `liblogos_core.so` that simulates both plugins in-process:

```bash
make demo-logos-core
```

### Run with real Logos Core SDK

```bash
LOGOS_CORE_LIB_DIR=/path/to/sdk/lib make demo-logos-core-real
```

### Expected output

```
╔══════════════════════════════════════════════════════════════╗
║  LMAO — Logos Core E2E Demo                                ║
║  Transport: LogosCoreDeliveryTransport (delivery_module)    ║
║  Storage:   LogosCoreStorageBackend   (storage_module)      ║
╚══════════════════════════════════════════════════════════════╝

[core] Logos Core initialized (headless / local mode)
[core] Loaded plugins: delivery_module, storage_module

── Step 1: Agent A uploads payload to Logos Storage ──────────
  Payload size: 131072 bytes
  Uploaded → CID: zStub0000

── Step 2: Agent A sends task to Agent B ─────────────────────
  → Sent via delivery_module IPC

── Step 3: Agent B receives task + downloads payload ─────────
  Received 1 task(s)
  Downloaded 131072 bytes from storage
  Payload integrity verified
  → Responded: "Processed payload (131072 bytes). All good!"

── Step 4: Agent A receives response ─────────────────────────
  Received 1 response(s)
  Response: "Processed payload (131072 bytes). All good!"
```

### What it proves

- `LogosCoreDeliveryTransport` correctly calls `delivery_module` via `logos_core_call_plugin_method_async`
- `LogosCoreStorageBackend` correctly uploads/downloads via `storage_module` with chunked transfer
- Both backends compile, link, and run against the Logos Core C API
- The full A2A flow works: announce → send task with storage CID → receive → download → respond

## Crates

| Crate | Description |
|-------|-------------|
| `logos-messaging-a2a-crypto` | X25519 ECDH + ChaCha20-Poly1305 encryption |
| `logos-messaging-a2a-core` | A2A types: `AgentCard`, `Task`, `Message`, `Part` |
| `logos-messaging-a2a-transport` | `Transport` trait + nwaku REST + `InMemoryTransport` + `LogosCoreDeliveryTransport` + SDS reliability |
| `logos-messaging-a2a-storage` | `StorageBackend` trait + Logos Storage (Codex) REST + `LogosCoreStorageBackend` |
| `logos-messaging-a2a-node` | A2A node: announce, discover, send/receive tasks, presence, payments |
| `logos-messaging-a2a-execution` | `ExecutionBackend` trait + Status Network (EVM) + LEZ backends for on-chain payments & agent registration |
| `logos-messaging-a2a-cli` | CLI for interacting with the network |
| `logos-messaging-a2a-mcp` | MCP bridge — expose agents as tools for Claude, Cursor, etc. |
| `logos-messaging-a2a-ffi` | C FFI bridge for Logos Core Qt module integration |
| `lmao-ffi` | High-level C FFI wrapper (simpler API for embedders) |
| `logos-messaging-a2a-execution` | On-chain execution: `ExecutionBackend` trait + Status Network (EVM) + LEZ stub |

## Encryption

End-to-end encrypted using **X25519 ECDH + ChaCha20-Poly1305** (stepping stone).
Future: [Logos Chat SDK](https://github.com/nicola/logos-chat-sdk) with Double Ratchet for forward secrecy.

## Storage Offloading

When a message payload exceeds a configurable threshold (default 64 KB), LMAO
automatically offloads it to Logos Storage (Codex) and sends only the CID in
the Waku envelope. The receiver fetches the full payload by CID transparently.

```rust
use logos_messaging_a2a_node::{StorageOffloadConfig, WakuA2ANode};
use logos_messaging_a2a_storage::StorageBackend;
use std::sync::Arc;

// Any StorageBackend impl works: LogosStorageRest, LibstorageBackend, etc.
let storage: Arc<dyn StorageBackend> = /* your backend */;

let node = WakuA2ANode::new("agent", "my agent", vec![], transport)
    .with_storage_offload(StorageOffloadConfig::new(storage));
// Large payloads are now offloaded automatically on send and fetched on receive.
```

## Presence Discovery

Agents announce themselves on a well-known Waku topic (`/lmao/1/presence/proto`).
Other agents subscribe, build a live `PeerMap`, and query it by capability when
routing tasks — no central registry needed.

```rust
use logos_messaging_a2a_transport::memory::InMemoryTransport;
use logos_messaging_a2a_node::WakuA2ANode;

// Create two agents on a shared transport
let transport = InMemoryTransport::new();
let alice = WakuA2ANode::new("alice", "Alice agent", vec!["summarize".into()], transport.clone());
let bob = WakuA2ANode::new("bob", "Bob agent", vec!["code".into()], transport.clone());

// Alice announces presence (TTL = 5 min by default)
alice.announce_presence().await?;

// Bob polls presence and discovers Alice
bob.poll_presence().await?;
let peers = bob.find_peers_by_capability("summarize");
assert_eq!(peers.len(), 1);
assert_eq!(peers[0].1.name, "alice");
```

The `PeerMap` lazily evicts expired entries. Call `peers().evict_expired()` to
clean up, or just rely on `get()` / `find_by_capability()` which skip expired
entries automatically.

## x402 Payment Flow

LMAO supports [x402-style](https://www.x402.org/) payment gating: agents can
require payment before processing tasks, and senders can auto-pay via an
`ExecutionBackend`.

```rust
use logos_messaging_a2a_node::{PaymentConfig, WakuA2ANode};
use std::sync::Arc;

// Receiver: require 100 tokens per task, verify on-chain
let receiver = WakuA2ANode::new("service", "Paid service", vec![], transport.clone())
    .with_payment(PaymentConfig {
        backend: backend.clone(),
        required_amount: 100,
        auto_pay: false,
        auto_pay_amount: 0,
        verify_on_chain: true,
        receiving_account: "0xmy_wallet".to_string(),
    });

// Sender: auto-pay 100 tokens on every outgoing task
let sender = WakuA2ANode::new("client", "Client", vec![], transport.clone())
    .with_payment(PaymentConfig {
        backend: backend.clone(),
        required_amount: 0,
        auto_pay: true,
        auto_pay_amount: 100,
        verify_on_chain: false,
        receiving_account: String::new(),
    });
```

**Security features:**
- **Replay protection** — each tx hash can only be used once
- **On-chain verification** — optionally verify amount + recipient via `ExecutionBackend`
- **Offline mode** — trust claimed amounts when on-chain verification is disabled

Currently supported backends: `StatusNetworkBackend` (Status Network Sepolia).
`LezExecutionBackend` is stubbed for future LEZ chain support.

## Testing

All transport implementations are swappable via the `Transport` trait.
For unit/integration tests, use `InMemoryTransport` — no Logos Messaging node required:

```rust
use logos_messaging_a2a_transport::InMemoryTransport;
use std::sync::Arc;

let transport = Arc::new(InMemoryTransport::new());
// Pass to WakuA2ANode — agents communicate in-process
```

## MCP Bridge

Expose your Logos Messaging agent fleet as MCP tools usable by Claude Desktop, Cursor, or any MCP-compatible host:

```json
// Claude Desktop config (~/.config/claude/claude_desktop_config.json)
{
  "mcpServers": {
    "logos-agents": {
      "command": "logos-messaging-a2a-mcp",
      "args": ["--node-url", "http://localhost:8645"]
    }
  }
}
```

Tools exposed:
- `discover_agents` — list all agents on the network
- `send_to_agent` — send a task to a specific agent and get a response
- `list_cached_agents` — show last known agents without a network call

## Logos Core Module

The `logos-messaging-a2a-ffi` crate and `module/` directory provide a Logos Core Qt plugin
(IComponent-based) for embedding the A2A agent fleet UI directly in the Logos desktop app.

```
module/
  qml/MessagingA2AView.qml    # Agent fleet UI
  src/MessagingA2ABackend.cpp # Qt backend
  src/MessagingA2AUIComponent.cpp # IComponent entry point
  CMakeLists.txt
```

## Roadmap

- [x] Core A2A types (AgentCard, Task, Message)
- [x] nwaku REST transport
- [x] X25519 + ChaCha20-Poly1305 encryption
- [x] SDS reliability layer
- [x] MCP bridge (Claude Desktop, Cursor)
- [x] C FFI for Logos Core integration
- [x] Qt IComponent module scaffolding
- [x] InMemoryTransport for testing + CI
- [x] `LogosCoreDeliveryTransport` — native delivery_module IPC transport
- [x] `LogosCoreStorageBackend` — native storage_module IPC backend
- [x] Logos Core e2e demo (stub + real SDK support)
- [x] libwaku FFI —  via  feature (no separate nwaku process)
- [x] CID-based large payload offloading to Logos Storage
- [x] Full SDS protocol — bloom filters, causal ordering, batch ACK, repair requests
- [x] Waku presence broadcasts — PeerMap discovery via well-known topic
- [x] x402 payment flow — auto-pay, payment gating, on-chain verification, replay protection
- [x] End-to-end demo — two agents, one task, payment flow, InMemoryTransport
- [ ] Logos Chat SDK — Double Ratchet for forward secrecy
- [ ] LEZ agent registry — on-chain AgentCards via SPELbook
- [ ] Logos Core plugin — packaged `.lgx` module

## Part of the SPEL Ecosystem

| Repo | Description |
|------|-------------|
| [spel](https://github.com/jimmy-claw/spel) | Smart Program Execution Layer — LEZ framework |
| [spelbook](https://github.com/jimmy-claw/spelbook) | On-chain program registry |
| [lez-multisig-framework](https://github.com/jimmy-claw/lez-multisig-framework) | Multisig governance |
| [lmao](https://github.com/jimmy-claw/lmao) | This repo — A2A agent orchestration |

## License

MIT
