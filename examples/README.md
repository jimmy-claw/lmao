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

## echo_agent — Simple Echo

Single agent that echoes back any message it receives.

```bash
cargo run --example echo_agent
```
