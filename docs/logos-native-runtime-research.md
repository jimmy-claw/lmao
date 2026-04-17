# Research: Logos-Native Agent Runtime

Issue: #51 — Replacing OpenClaw with a Logos Core module

## Executive Summary

LMAO already has most building blocks for a Logos-native agent runtime. The gap
is not infrastructure — it is the **agent lifecycle layer** (tool execution,
memory, heartbeats, scheduling) that OpenClaw provides today. This document maps
what exists, what is missing, and recommends a phased approach.

## Current State (what LMAO already provides)

| Capability | LMAO Crate | Status |
|---|---|---|
| Agent identity (secp256k1) | `lmao-core` | Done |
| Agent discovery (Waku broadcast + presence) | `lmao-node` (discovery/presence) | Done |
| Agent-to-agent messaging | `lmao-node` (send_task/respond) | Done |
| E2E encryption | `lmao-crypto` (X25519+ChaCha20) | Done |
| Reliable delivery (full SDS) | `lmao-transport` (bloom filter, causal ordering, batch ACK) | Done |
| Task delegation (4 strategies) | `lmao-node` (FirstAvailable, CapabilityMatch, BroadcastCollect, RoundRobin) | Done |
| Task streaming | `lmao-node` | Done |
| On-chain registration | `lmao-execution` (StatusNetwork) | Done |
| On-chain payments (x402) | `lmao-execution` (StatusNetwork) | Done |
| ZK execution (LEZ) | `lmao-execution` (lez module) | Stub |
| CID payload offloading | `lmao-storage` (Codex REST + LogosCore IPC) | Done |
| Logos Core IPC transport | `lmao-transport` (logos-core feature) | Done |
| Logos Core UI plugin | `logos-core-module/` (IComponent + QML UI) | Done (PR #43) |
| MCP bridge (Claude/Cursor) | `lmao-mcp` | Done |
| C FFI bindings | `lmao-ffi` + `logos-messaging-a2a-ffi` | Done |
| CLI with JSON output | `lmao-cli` (info, health, sessions, completions) | Done |
| Observability | `lmao-node` (metrics counters) | Done |
| Message retry with backoff | `lmao-node` (RetryLayer) | Done |

## What OpenClaw provides that LMAO does not

| OpenClaw Feature | Logos-Native Equivalent | Gap |
|---|---|---|
| Telegram bridge | Logos Messaging (Waku) | Waku works for A2A; no human chat UI yet |
| File-based memory | Logos Storage (Codex CID) | `StorageBackend` trait exists, Codex + LogosCore impl done |
| Cron scheduling | Waku-triggered events | No scheduler in LMAO — needs new component |
| Tool execution sandbox | — | Not in LMAO at all |
| Browser automation | — | Not in LMAO at all |
| Heartbeat system | `lmao-node` presence module | Done (PeerMap + signed broadcasts + TTL expiry) |
| LLM API calls | — | Anthropic API (unavoidable for now) |
| Agent spawn/lifecycle | Logos Core module loading | Partial — load module, but no spawn API |

### Critical Gaps

1. **Agent Lifecycle Manager** — start, stop, restart, health-check agents.
   Today the Logos Core module can load LMAO, but there is no API to spawn a
   *new* agent instance with its own identity and config.

2. **Scheduler / Trigger System** — OpenClaw uses cron. A Logos-native approach
   would use Waku content-topic triggers or on-chain events. Neither exists in
   LMAO today.

3. **Tool Execution Sandbox** — OpenClaw has shell execution, browser control,
   file I/O. A Logos-native agent would need a sandboxed execution environment
   (WASM? Nix sandbox?). This is the hardest gap to close.

4. **LLM Integration Layer** — LMAO has no concept of "call an LLM". The agent
   runtime needs a trait for LLM backends (Anthropic today, local models later).

5. **Human-Facing Comms** — Waku works for machine-to-machine. Humans use
   Telegram/Discord today. Until Logos Messaging has a consumer chat client, the
   Telegram bridge remains necessary.

## Architecture: Proposed Runtime Layer

```
┌─────────────────────────────────────────────────────────┐
│                   Agent Runtime (NEW)                     │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Lifecycle Mgr │  │  Scheduler   │  │  Tool Runner │  │
│  │ spawn/stop/   │  │ cron/waku    │  │  sandboxed   │  │
│  │ health-check  │  │ triggers     │  │  execution   │  │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  │
│         │                 │                  │           │
│  ┌──────┴─────────────────┴──────────────────┴───────┐  │
│  │              LLM Integration Trait                 │  │
│  │  AnthropicBackend | LocalModelBackend | ...        │  │
│  └──────────────────────┬────────────────────────────┘  │
│                         │                                │
├─────────────────────────┼────────────────────────────────┤
│              LMAO (existing infrastructure)               │
│                                                          │
│  WakuA2ANode ─── transport, crypto, storage, execution   │
│  ├── SdsTransport (bloom filter dedup, causal ordering)  │
│  ├── PeerMap (signed presence, capability match)         │
│  ├── Delegation (4 strategies incl. RoundRobin)          │
│  ├── RetryLayer (exponential backoff + jitter)           │
│  ├── Metrics (observability counters)                    │
│  └── MCP Bridge (Claude/Cursor integration)              │
│                                                          │
├──────────────────────────────────────────────────────────┤
│              Logos Core (host)                            │
│  IComponent plugin loading                               │
│  delivery_module IPC (pub/sub)                           │
│  storage_module IPC (CID upload/download)                │
└──────────────────────────────────────────────────────────┘
```

## Existing Integration Points (code audit)

### Transport: Two production-ready paths

1. **NwakuRestTransport** — REST API to external nwaku node (`http://localhost:8645`)
2. **LogosCoreDeliveryTransport** — Native IPC via `delivery_module` plugin

Both implement `WakuTransport { publish, subscribe, poll }`. The Logos Core
transport is already battle-tested in the e2e demo (`demos/logos-core-e2e/`).

### Storage: CID-addressed with auto-offload

```
StorageBackend { upload(data) → CID, download(CID) → data }
├── LogosStorageRest        — Codex REST API (standalone processes)
├── LogosCoreStorageBackend — storage_module IPC (inside Logos Core)
└── LibstorageBackend       — Direct FFI (future)
```

Payloads > 100 KB are auto-offloaded to storage via `maybe_offload()`. This
means agent memory/context could use CID-addressed blobs with zero new code.

### Execution: On-chain agent economy

```
ExecutionBackend { register_agent, pay, balance, verify_transfer }
├── StatusNetworkBackend — EVM/gasless (AgentRegistry at 0x438bB48f...)
└── LezExecutionBackend  — ZK-verified (stub, tracking issue #4)
```

The x402 payment flow already enables pay-per-task agent economics.

### FFI: Three layers deep

```
Logos Core Qt App
  → LmaoComponent (IComponent C++ plugin)
    → LmaoBackend (QObject, calls C FFI)
      → lmao-ffi (Rust cdylib, JSON in/out)
        → WakuA2ANode (full async Rust stack)
```

Key FFI functions: `lmao_discover_agents()`, `lmao_send_task()`,
`lmao_get_agent_card()`. All return `{"success": bool, ...}` JSON.

## Phased Approach

### Phase 1: Hybrid (recommended NOW)

Keep OpenClaw for human-facing operations. Use LMAO for agent-to-agent
coordination underneath.

- OpenClaw handles: Telegram, crons, memory, tool execution
- LMAO handles: agent discovery, A2A messaging, on-chain identity/payments
- Bridge: MCP server (`lmao-mcp`) connects Claude <-> LMAO agents

**Already possible today.** The MCP bridge and FFI bindings enable this.

**Concrete integration path:**
1. OpenClaw agent registers on-chain via `lmao-execution` (StatusNetwork)
2. OpenClaw agent announces presence via `lmao-node` (Waku broadcast)
3. Other LMAO agents discover it and send tasks via A2A protocol
4. OpenClaw processes tasks using its existing tool execution pipeline
5. Results flow back through LMAO transport

### Phase 2: Agent Lifecycle in Logos Core

Add a new crate `logos-messaging-a2a-runtime` that provides:

```rust
/// Trait for agent runtime backends.
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Spawn a new agent with the given config.
    async fn spawn(&self, config: AgentConfig) -> Result<AgentHandle>;
    /// Stop a running agent.
    async fn stop(&self, id: &AgentId) -> Result<()>;
    /// List running agents with status.
    async fn list(&self) -> Result<Vec<AgentStatus>>;
    /// Health check a specific agent.
    async fn health(&self, id: &AgentId) -> Result<HealthStatus>;
}

pub struct AgentConfig {
    pub name: String,
    pub capabilities: Vec<String>,
    pub llm_backend: LlmBackendConfig,
    pub triggers: Vec<TriggerConfig>,
    pub tools: Vec<ToolConfig>,
    pub storage_backend: StorageBackendConfig,
}

/// Trait for LLM backends (Anthropic today, local models later).
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(&self, messages: Vec<Message>) -> Result<Message>;
    async fn complete_stream(&self, messages: Vec<Message>) -> Result<MessageStream>;
}
```

This enables "spin up agent = load a module" in Basecamp.

**Prerequisite:** Logos Core module loading stabilizes for production workloads.

### Phase 3: Full Logos-Native

Replace OpenClaw entirely when:

- [ ] Logos Messaging has a human-facing chat client (Telegram replacement)
- [ ] LEZ execution backend is production-ready (issue #4)
- [ ] Tool execution sandbox exists (WASM or Nix-based)
- [ ] Logos Storage (Codex) is reliable for agent memory persistence
- [ ] Logos Core supports long-running module processes with restart policies

This is the LP-0008 end state.

## Risk Assessment

| Risk | Impact | Likelihood | Mitigation |
|---|---|---|---|
| Logos Core instability | Agent crashes, data loss | Medium | Phase 1 hybrid keeps OpenClaw as fallback |
| No human chat replacement | Can't drop Telegram bridge | High | Keep Telegram bridge until Status Chat/Waku consumer client |
| LLM vendor lock-in | Anthropic dependency | Low (acceptable) | LLM trait allows swapping backends later |
| Tool sandbox complexity | Months of work | High | Start with simple subprocess isolation, not full WASM |
| LEZ delays | No ZK-verified execution | Medium | StatusNetwork EVM works today as bridge |

## Recommendations

1. **Do NOT replace OpenClaw now.** The hybrid approach (Phase 1) gives immediate
   value without risk.

2. **Next concrete step:** Create `logos-messaging-a2a-runtime` crate with the
   `AgentRuntime` and `LlmBackend` traits and a basic in-process implementation.
   This is the foundation for Phase 2.

3. **Leverage what exists:** The delegation system (4 strategies including
   RoundRobin) already supports orchestrator patterns. An OpenClaw agent could
   act as orchestrator, delegating subtasks to specialized LMAO agents.

4. **Track blockers externally:**
   - Logos Core production stability -> Logos team
   - LEZ SDK availability -> LEZ team
   - Logos Messaging consumer client -> Status/Waku team

5. **The MCP bridge is the most underrated piece.** It already lets any
   MCP-compatible AI (Claude, Cursor) use LMAO agents as tools. This is the
   practical integration point for Phase 1.

## Related Issues & PRs

- PR #43 — Logos Core IComponent module (done)
- Issue #5 — Package as .lgx (done)
- Issue #4 — LEZ agent registry (stub exists)
- LP-0008 spec — Full Logos-native runtime (end state)
- AgentRegistry contract: `0x438bB48f...` on SN testnet
