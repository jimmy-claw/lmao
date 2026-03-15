# LMAO Examples

## two_agents — End-to-End Demo

Two agents discover each other, negotiate a task with payment, and deliver results — all peer-to-peer over Waku topics with zero HTTP servers.

**What happens:**

1. **Agent A** (requester) and **Agent B** (worker) announce themselves on the discovery topic
2. Agent A discovers Agent B by capability (`summarization`)
3. Agent A sends a "summarize this text" task, auto-paying 100 tokens
4. Agent B verifies the payment meets its 50-token minimum, then processes
5. Agent B responds with the summary
6. Agent A receives the result

**Run it:**

```bash
cargo run --example two_agents
```

No external dependencies — uses `InMemoryTransport` and a mock payment backend.

## ping_pong — Basic Message Exchange

Two agents exchange ping/pong messages. Supports `--encrypt` for X25519+ChaCha20-Poly1305 end-to-end encryption.

```bash
cargo run --example ping_pong
cargo run --example ping_pong -- --encrypt
```

## presence_discovery — Full Agent Lifecycle

Demonstrates the complete LMAO agent lifecycle: presence broadcast, peer discovery via the signed peer map, capability-based lookup, and a full task round-trip between two agents.

**What happens:**

1. **Alice** and **Bob** are created with `InMemoryTransport`
2. Both agents broadcast signed presence announcements (`announce_presence`)
3. Both agents poll presence (`poll_presence`) and discover each other in the peer map
4. Alice finds Bob by capability (`summarization`) using `find_peers_by_capability`
5. Alice sends a task to Bob
6. Bob processes and responds
7. Alice receives the response — full round-trip complete

```bash
cargo run --example presence_discovery
```

## echo_agent — Simple Echo

Single agent that echoes back any message it receives.

```bash
cargo run --example echo_agent
```
