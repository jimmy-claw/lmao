# LMAO — Logos Module for Agent Orchestration

> **LMAO** = **L**ogos **M**odule for **A**gent **O**rchestration
>
> Yes, the acronym is intentional. Building decentralized AI agent infrastructure is serious work — but it doesn't have to be humourless. LMAO implements Google's [A2A protocol](https://github.com/google/A2A) over [Waku](https://waku.org/) decentralized transport, bringing censorship-resistant, serverless agent-to-agent communication to the Logos stack.

## The Problem

Google's A2A protocol is great. But it assumes HTTP: stable endpoints, central registries, easy censorship. That's fine for web2. For a decentralized agent network running on Logos, it's a non-starter.

**LMAO** replaces HTTP with Waku — a decentralized pub/sub network — giving you full A2A semantics with:

| | HTTP/SSE | LMAO (Waku) |
|---|---|---|
| Discovery | Central registry | Content-addressed pub/sub topics |
| Endpoints | Stable IP required | Just a pubkey |
| Privacy | Traffic analysis easy | Optional E2E encryption |
| Censorship | Single point of failure | Decentralized relay |
| NAT | Needs port forwarding | Works behind NAT |

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  Waku Relay Network                  │
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
      ┌────▼──────────────┐
      │  MCP Bridge       │  ← Claude Desktop / Cursor
      │  waku-a2a-mcp     │     can talk to any agent
      └───────────────────┘
           │
      ┌────▼──────────────┐
      │  Logos Core       │  ← Qt UI plugin
      │  waku-a2a-ffi     │     for the Logos desktop app
      └───────────────────┘
```

## Quick Start

```bash
# Ping-pong demo — no nwaku needed, fully in-memory
cargo run --example ping_pong

# With encryption
cargo run --example ping_pong -- --encrypt

# MCP bridge (requires nwaku running on :8645)
cargo run -p waku-a2a-mcp -- --waku-url http://localhost:8645
```

## Crates

| Crate | Description |
|-------|-------------|
| `waku-a2a-crypto` | X25519 ECDH + ChaCha20-Poly1305 encryption |
| `waku-a2a-core` | A2A types: `AgentCard`, `Task`, `Message`, `Part` |
| `waku-a2a-transport` | `WakuTransport` trait + nwaku REST + `InMemoryTransport` (testing) + SDS reliability layer |
| `waku-a2a-node` | A2A node: announce, discover, send/receive tasks |
| `waku-a2a-cli` | CLI for interacting with the network |
| `waku-a2a-mcp` | ✅ MCP bridge — expose agents as tools for Claude, Cursor, etc. |
| `waku-a2a-ffi` | C FFI bridge for Logos Core Qt module integration |
| `lmao-ffi` | High-level C FFI wrapper (simpler API for embedders) |

## Encryption

End-to-end encrypted using **X25519 ECDH + ChaCha20-Poly1305** (stepping stone).
Future: [Logos Chat SDK](https://github.com/nicola/logos-chat-sdk) with Double Ratchet for forward secrecy.

## Testing

All transport implementations are swappable via the `WakuTransport` trait.
For unit/integration tests, use `InMemoryTransport` — no Waku network required:

```rust
use waku_a2a_transport::InMemoryTransport;
use std::sync::Arc;

let transport = Arc::new(InMemoryTransport::new());
// Pass to WakuA2ANode — agents communicate in-process
```

## MCP Bridge

Expose your Waku agent fleet as MCP tools usable by Claude Desktop, Cursor, or any MCP-compatible host:

```json
// Claude Desktop config (~/.config/claude/claude_desktop_config.json)
{
  "mcpServers": {
    "logos-agents": {
      "command": "waku-a2a-mcp",
      "args": ["--waku-url", "http://localhost:8645"]
    }
  }
}
```

Tools exposed:
- `discover_agents` — list all agents on the network
- `send_to_agent` — send a task to a specific agent and get a response
- `list_cached_agents` — show last known agents without a network call

## Logos Core Module

The `waku-a2a-ffi` crate and `module/` directory provide a Logos Core Qt plugin
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
