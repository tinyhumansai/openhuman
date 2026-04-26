# MCP-style transport (`app/src/lib/mcp/`)

## What this is

The **`app/src/lib/mcp/`** folder provides **shared utilities** for structured, JSON-RPC–style messages over **Socket.io**: types, transport, validation, rate limiting, and error handling.

It does **not** currently ship a large `telegram/tools/` tree or dozens of hand-written Telegram MCP tools. Agent tooling is provided through:

- The **skills** runtime in the Rust core (`openhuman` binary, QuickJS skills), and
- Backend/agent integration that lists tools via realtime sync (`tool:sync` and related flows — see [`../ARCHITECTURE.md`](../ARCHITECTURE.md)).

## Layout

```
app/src/lib/mcp/
├── index.ts           # Public exports
├── types.ts           # Shared interfaces
├── transport.ts       # Socket.io JSON-RPC transport helpers
├── validation.ts      # Input validation helpers
├── rateLimiter.ts
├── errorHandler.ts
├── logger.ts
└── __tests__/         # Vitest unit tests
```

## Transport (`transport.ts`)

Used to send/receive JSON-RPC 2.0–style payloads over the existing Socket.io connection (timeouts, pending request maps, etc.). Exact method names and prefixes depend on the server contract; align changes with the backend and the Rust core.

## Types (`types.ts`)

Shared TypeScript types for tools and MCP messages. Keep these aligned with whatever the socket server and core emit.

## Testing

Vitest tests live next to the code under `__tests__/`. Run from the repo root:

```bash
pnpm workspace openhuman-app test
```

## See also

- High-level MCP + AI flow: [`../ARCHITECTURE.md`](../ARCHITECTURE.md) (AI & tool protocol)
- Socket layer: [Services](./03-services.md) (`socketService`)
