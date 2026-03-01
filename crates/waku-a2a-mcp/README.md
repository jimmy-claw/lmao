# waku-a2a-mcp — MCP Bridge for Logos Messaging

Expose agents on the Logos/Waku decentralized network as [MCP](https://modelcontextprotocol.io) tools,
usable from Claude Desktop, Cursor, VS Code, and any MCP-compatible host.

## Architecture

```
MCP Host (Claude Desktop)
    │ stdio (JSON-RPC)
    ▼
┌──────────────────┐
│  waku-a2a-mcp    │  ← this crate
│  (MCP Server)    │
└────────┬─────────┘
         │ HTTP REST
         ▼
┌──────────────────┐
│   nwaku node     │
│  (Waku network)  │
└────────┬─────────┘
         │ Waku relay
         ▼
┌──────────────────┐
│  Agent Fleet     │
│  (echo, RAG, …)  │
└──────────────────┘
```

## MCP Tools Exposed

| Tool | Description |
|------|-------------|
| `discover_agents` | Scan the Waku network for announced A2A agents |
| `send_to_agent` | Send a message/task to a specific agent by name |
| `list_cached_agents` | List agents from last discovery (no network call) |

## Usage

### Standalone
```bash
cargo run -p waku-a2a-mcp -- --waku-url http://localhost:8645
```

### Claude Desktop config (`claude_desktop_config.json`)
```json
{
  "mcpServers": {
    "logos-agents": {
      "command": "waku-a2a-mcp",
      "args": ["--waku-url", "http://localhost:8645"]
    }
  }
}
```

### OpenClaw / Cursor
Add to your MCP server config pointing to the binary.

## Options

- `--waku-url <URL>` — nwaku REST API endpoint (default: `http://localhost:8645`)
- `--timeout <SECS>` — max wait for agent responses (default: 30)

## Dependencies

- [rmcp](https://crates.io/crates/rmcp) — Official Rust MCP SDK
- waku-a2a-node — A2A protocol layer
- nwaku — Waku network node (must be running)
