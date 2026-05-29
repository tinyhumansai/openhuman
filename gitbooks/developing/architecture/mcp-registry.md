---
description: >-
  The dynamic, user-facing side of MCP-client support — discover servers on
  Smithery.ai, persist installs to SQLite, supervise local-spawn subprocess
  lifecycle, surface their tools to agents via the unified tool registry.
icon: plug
---

# MCP Registry (`src/openhuman/mcp_registry/`)

`src/openhuman/mcp_registry/` is the **dynamic, user-facing** half of OpenHuman's Model Context Protocol client support. It lets a user browse the Smithery.ai MCP registry, install a chosen server, persist that choice to SQLite, and (for servers launched as local subprocesses) supervise the subprocess lifecycle. Installed servers' tools are surfaced to agents via the unified tool registry (`crate::openhuman::tool_registry`).

> **Naming note**: the Rust module path is `mcp_registry`, but the RPC namespace and on-disk SQLite filename are still `mcp_clients` for backward compatibility with existing frontend code and stored user state. Grep both names when chasing call sites.

This module is paired with `src/openhuman/mcp_client/` — the **transport library** (HTTP + stdio primitives) plus the _static, config-declared_ server set read from `[[mcp_client.servers]]` in `config.toml`. Agents reach that static set through generic bridge tools. The static set is intentionally separate from this dynamic registry; both kinds will eventually share the transport primitives from `mcp_client`.

```text
                 ┌───────────────────────────────────────────────┐
   Smithery.ai ──► registries/ + registry.rs (10-min SQLite cache)│
                 └────────────────────┬──────────────────────────┘
                                      │ browse / install
                                      ▼
                          ┌──────────────────────┐
   Frontend (Skills UI) ─►│  ops.rs / schemas.rs │  RPC controllers
                          └──────────┬───────────┘
                                     │
                                     ▼
                          ┌──────────────────────┐
                          │      store.rs        │  mcp_clients.db (SQLite)
                          │  InstalledServer rows│
                          └──────────┬───────────┘
                                     │ at boot
                                     ▼
                          ┌──────────────────────┐
                          │       boot.rs        │  spawn_installed_servers
                          └──────────┬───────────┘
                                     │ for each local-spawn
                                     ▼
                          ┌──────────────────────┐
                          │   connections.rs     │  wraps mcp_client::
                          │  (global registry)   │  McpStdioClient
                          └──────────┬───────────┘
                                     │ surfaces tools to
                                     ▼
                          tool_registry (agents)
```

## Server transport model

Today every `InstalledServer` is a **local subprocess** launched by `npx`, `uvx`, or a direct binary (see `types::CommandKind`). The connection is **stdio JSON-RPC**, owned by `connections.rs`.

HTTP-remote MCP servers (the majority of what Smithery actually lists) are **not yet modelled** as an `InstalledServer` variant. Adding a remote transport variant is planned follow-up work; after that, the registry will hold both kinds and `connections.rs` will dispatch by transport.

## Boot-time spawn

`boot::spawn_installed_servers` is called from `bootstrap_core_runtime` so every local-spawn server is connected as soon as the core comes up. Errors are logged per-server and **never block boot** — a broken MCP install should not gate the desktop app starting.

## Layout

| Path                        | Role                                                                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `types.rs`                  | Data structures: `InstalledServer`, `McpTool`, `ConnStatus`, Smithery DTOs, etc.                                                                       |
| `store.rs`                  | SQLite persistence — `mcp_clients.db`, CRUD over `InstalledServer` rows.                                                                               |
| `registry.rs`               | Smithery HTTP client with a 10-minute SQLite cache so re-browsing doesn't hammer the upstream registry.                                                |
| `registries/`               | Adapters for the upstream registries this code can browse (currently Smithery).                                                                        |
| `connections.rs`            | Global in-process connection registry. Wraps `crate::openhuman::mcp_client::McpStdioClient` — there is no separate stdio client implementation here.   |
| `boot.rs`                   | Boot-time spawn (`spawn_installed_servers`) called from `bootstrap_core_runtime`.                                                                      |
| `setup.rs` / `setup_ops.rs` | "Setup agent" support — the small agent that walks a user through configuring a freshly installed server (env vars, secrets, first connect).           |
| `ops.rs`                    | RPC handler implementations (install, uninstall, list, browse, enable / disable, etc.).                                                                |
| `schemas.rs`                | Controller schemas + handler dispatch. Re-exported from `mod.rs` as `all_mcp_registry_controller_schemas` / `all_mcp_registry_registered_controllers`. |
| `bus.rs`                    | `DomainEvent` subscriber for lifecycle logging.                                                                                                        |

## Public surface

The exports from `mod.rs` are intentionally narrow:

```rust
pub use schemas::{
    all_controller_schemas as all_mcp_registry_controller_schemas,
    all_registered_controllers as all_mcp_registry_registered_controllers,
    schemas as mcp_registry_schemas,
};

pub use types::{ConnStatus, InstalledServer, McpTool};
```

Everything else — `boot`, `bus`, `connections`, `store`, `setup`, `setup_ops` — is `pub mod` for in-crate callers but not re-exported.

## Calls into

- `crate::openhuman::mcp_client::McpStdioClient` — the actual stdio transport.
- `crate::openhuman::tool_registry` — installed servers' tools land here so agents see them alongside native tools.
- `memory_store` / workspace SQLite — for `mcp_clients.db` persistence.
- Smithery.ai HTTP — registry browsing.

## Called by

- `bootstrap_core_runtime` (via `boot::spawn_installed_servers`).
- Frontend Skills UI — the (currently stubbed) MCP servers panel will dispatch through `ops.rs` over the `openhuman.mcp_clients_*` RPC namespace.
- The setup agent in `setup_ops.rs` — for first-connect onboarding.

## Tests

Unit tests are co-located inline under `#[cfg(test)]` blocks in `store.rs`, `connections.rs`, and `setup.rs`. There is no dedicated `*_tests.rs` sibling per file (the convention in this domain is inline).

## Related

- [`mcp_registry/mod.rs`](https://github.com/tinyhumansai/openhuman/blob/main/src/openhuman/mcp_registry/mod.rs) — the authoritative rustdoc this page mirrors.
- `src/openhuman/mcp_client/` — the transport library + static config-declared server set.
- [Agent Harness](agent-harness.md) — how the agent ends up calling MCP tools through `tool_registry`.
- [Architecture overview](../architecture.md) — where this fits in the wider system.
