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
```

## Crates

| Crate | Description |
|-------|-------------|
| `logos-messaging-a2a-crypto` | X25519 ECDH + ChaCha20-Poly1305 encryption |
| `logos-messaging-a2a-core` | A2A types: `AgentCard`, `Task`, `Message`, `Part` |
| `logos-messaging-a2a-transport` | `Transport` trait + nwaku REST (`LogosMessagingTransport`) + `InMemoryTransport` (testing) + SDS reliability layer |
| `logos-messaging-a2a-node` | A2A node: announce, discover, send/receive tasks |
| `logos-messaging-a2a-cli` | CLI for interacting with the network |
| `logos-messaging-a2a-mcp` | MCP bridge — expose agents as tools for Claude, Cursor, etc. |
| `logos-messaging-a2a-ffi` | C FFI bridge for Logos Core Qt module integration |
| `lmao-ffi` | High-level C FFI wrapper (simpler API for embedders) |

## Encryption

End-to-end encrypted using **X25519 ECDH + ChaCha20-Poly1305** (stepping stone).
Future: [Logos Chat SDK](https://github.com/nicola/logos-chat-sdk) with Double Ratchet for forward secrecy.

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
- [ ] libwaku FFI — embedded libwaku (no separate nwaku process)
- [ ] Full SDS protocol — bloom filters, causal ordering, batch ACK
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
