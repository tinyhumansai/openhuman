# mcp_registry

The dynamic, user-facing side of MCP-client support. It browses upstream MCP server directories (Smithery.ai + the official modelcontextprotocol/registry), persists the user's chosen installs to SQLite, supervises their per-server connection lifecycle (stdio subprocess **or** HTTP-remote dial), and surfaces the connected servers' tools to agents via the unified tool registry. It also hosts a separate "setup agent" RPC surface (`mcp_setup`) that lets an LLM walk a non-technical user through search → secret collection → dry-run test → install-and-connect, with raw secret values kept out of the agent's context via opaque `secret://<hex>` refs.

The transport primitives themselves (stdio + HTTP MCP clients) live in the sibling `mcp_client` module — this module carries no transport code of its own, only lifecycle, dispatch, persistence, and the registry HTTP adapters. (Naming note: the RPC namespace and SQLite db filename are still `mcp_clients` for backwards-compat; the Rust module path is `mcp_registry`.)

## Responsibilities

- Search multiple upstream MCP registries in parallel and merge results; route detail fetches back to the originating registry.
- Persist installed-server records (no env values) + per-server env values + a TTL'd registry response cache in SQLite.
- Establish, track, and tear down live MCP connections keyed by `server_id`, dispatching on each install's `Transport` (stdio subprocess vs HTTP-remote).
- Spawn all installed local servers at boot without blocking core startup.
- Expose the install/connect/status/tool-call lifecycle over JSON-RPC (`mcp_clients_*`).
- Run a setup-agent surface (`mcp_setup_*`) with out-of-band secret collection so credentials never enter the LLM context.
- Provide an AI `config_assist` flow that guides users through filling required env vars.
- Publish lifecycle `DomainEvent`s for observability.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/mcp_registry/mod.rs` | Module docstring + exports; re-exports controller schema/registry pair and key types. |
| `src/openhuman/mcp_registry/types.rs` | Domain types: `CommandKind`, `Transport` (Stdio / HttpRemote), `InstalledServer`, `McpTool`, `ServerStatus`, `ConnStatus`, Smithery DTOs, `ChatTurn`. |
| `src/openhuman/mcp_registry/store.rs` | SQLite persistence (`mcp_clients/mcp_clients.db`): server CRUD, env values, registry cache; additive `transport`/`deployment_url` column migration. |
| `src/openhuman/mcp_registry/registry.rs` | Multi-registry dispatch: `registry_search` (parallel fan-out + merge), `registry_get` (source-prefix routing or first-hit). |
| `src/openhuman/mcp_registry/registries/mod.rs` | `Registry` trait + `enabled_registries` / `registry_for_source`; `SOURCE_*` constants. |
| `src/openhuman/mcp_registry/registries/smithery.rs` | Smithery.ai adapter (`registry.smithery.ai`), 10-min SQLite cache, optional `SMITHERY_API_KEY` auth. |
| `src/openhuman/mcp_registry/registries/mcp_official.rs` | Official modelcontextprotocol/registry adapter; cursor→page mapping with a bounded sequential cursor walk; optional `MCP_OFFICIAL_REGISTRY_*` env overrides. |
| `src/openhuman/mcp_registry/connections.rs` | Global in-process connection registry; `connect`/`disconnect`/`call_tool`/`all_status`/`all_connected_tools`; dispatches stdio vs HTTP via `ActiveClient`. |
| `src/openhuman/mcp_registry/boot.rs` | `spawn_installed_servers` — boot-time connect of all installs; per-server failures logged, never fatal. |
| `src/openhuman/mcp_registry/ops.rs` | `mcp_clients_*` RPC handler implementations + `resolve_command` + `config_assist` inference call. |
| `src/openhuman/mcp_registry/setup.rs` | Opaque secret-ref machinery (`SecretRef`, mint/fulfill/await/resolve/consume/gc) for the setup agent. |
| `src/openhuman/mcp_registry/setup_ops.rs` | `mcp_setup_*` RPC handlers (search/get/request_secret/submit_secret/test_connection/install_and_connect) + connection `pick_connection`. |
| `src/openhuman/mcp_registry/schemas.rs` | Controller schemas + `handle_*` dispatch for both `mcp_clients` and `mcp_setup` namespaces. |
| `src/openhuman/mcp_registry/bus.rs` | `McpClientEventSubscriber` — logs lifecycle events; `init()` registers it. |

## Public surface

From `mod.rs`:
- `all_mcp_registry_controller_schemas` / `all_mcp_registry_registered_controllers` (`schemas::all_controller_schemas` / `all_registered_controllers`).
- `mcp_registry_schemas` (`schemas::schemas`).
- Types `ConnStatus`, `InstalledServer`, `McpTool`.

`pub mod boot`, `bus`, `connections`, `setup`, `setup_ops`, `store`, `types` are public; `ops`, `registries`, `registry`, `schemas` are private (reached via the schema handlers). Notably `connections::all_connected_tools()` is consumed by `tool_registry`, and `boot::spawn_installed_servers` is called from core startup.

## RPC / controllers

Two namespaces. `all_controller_schemas()` returns 16 controllers (10 `mcp_clients` + 6 `mcp_setup`).

**`mcp_clients`** (`ops.rs`):
- `registry_search` — search the registries.
- `registry_get` — full server detail (augmented with `required_env_keys`).
- `installed_list` — list installed servers (env values omitted).
- `install` — install from registry (stdio-only legacy path); stores env values, publishes `McpServerInstalled`.
- `uninstall` — disconnect + delete.
- `connect` / `disconnect` — bring a server's connection up/down.
- `status` — per-server connection summaries.
- `tool_call` — invoke a tool on a connected server.
- `config_assist` — AI helper for filling required env vars (calls inference at `{api_url}/openai/v1/chat/completions`, falls back to a stub when unconfigured).

**`mcp_setup`** (`setup_ops.rs`):
- `search` / `get` — thin wrappers over `registry`.
- `request_secret` — mint a `secret://<hex>` ref, publish `McpSetupSecretRequested`, block up to 5 min for the UI to submit.
- `submit_secret` — UI-side fulfillment of a pending ref.
- `test_connection` — dry-run: dial a candidate (stdio scratch subprocess or HTTP-remote), list tools, tear down; nothing persisted.
- `install_and_connect` — commit: persist install + consume secret refs into `mcp_client_env`, then connect; returns `connected` or `installed_disconnected`.

## Agent tools

This module does not define `Tool` impls directly. The setup-agent tools live in `src/openhuman/tools/impl/network/mcp_setup.rs` as thin wrappers calling `mcp_registry::setup_ops`. The MCP browse/list/call tools (`McpListServersTool`, `McpListToolsTool`, etc.) are wired in `src/openhuman/tools/ops.rs` against the connected-tools surface, and `tool_registry` pulls live tools via `connections::all_connected_tools()`.

## Events

Publishes (via `publish_global`):
- `McpServerInstalled` (ops::install, setup_ops::install_and_connect)
- `McpServerConnected` (ops::connect)
- `McpServerDisconnected` (ops::disconnect)
- `McpClientToolExecuted` (ops::tool_call)
- `McpSetupSecretRequested` (setup_ops::request_secret)

Subscribes: `bus::McpClientEventSubscriber` (domain `"mcp_client"`) — logs the four `McpServer*` / `McpClientToolExecuted` events for observability; no side effects.

## Persistence

SQLite at `{workspace_dir}/mcp_clients/mcp_clients.db` (`store.rs`), three tables:
- `mcp_servers` — installed-server metadata (no env values); `transport`/`deployment_url` added via idempotent additive migration (pre-migration rows default to `stdio`).
- `mcp_client_env` — per-server env key/value pairs; values never serialized into any response and never logged. `ON DELETE CASCADE` from `mcp_servers`.
- `mcp_registry_cache` — registry HTTP response bodies with a 10-minute TTL.

Setup-agent secrets (`setup.rs`) are held in a **process-local in-memory map** (not SQLite) with a 5-min request timeout and 15-min idle GC; they only persist if committed via `consume_refs` → `mcp_client_env` during `install_and_connect`.

The in-process connection registry (`connections.rs`) is a `OnceLock<RwLock<HashMap<server_id, Connection>>>`, ephemeral per process.

## Dependencies

- `crate::openhuman::config::Config` — workspace dir, `mcp_client.client_identity`, inference api_url/api_key/default_model.
- `crate::openhuman::config::rpc` (`load_config_with_timeout`) — config load inside RPC handlers.
- `crate::openhuman::mcp_client` (`McpStdioClient`, `McpHttpClient`, `McpRemoteTool`, `McpServerToolResult`) — actual transport clients used by `connections.rs` and `setup_ops.rs`.
- `crate::core::event_bus` (`publish_global`, `DomainEvent`, `EventHandler`, `subscribe_global`) — lifecycle events + subscriber.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller wiring.
- `crate::rpc::RpcOutcome` — handler return contract.
- External crates: `rusqlite` (store), `reqwest` + `futures` (registry HTTP / fan-out), `tokio::sync` (RwLock/Mutex/oneshot), `uuid`, `async_trait`, `parking_lot` (official-registry cursor cache).

## Used by

- `src/openhuman/mod.rs` — declares `pub mod mcp_registry`.
- `src/core/all.rs` — registers `all_mcp_registry_registered_controllers()` + `all_mcp_registry_controller_schemas()`.
- `src/core/jsonrpc.rs` — calls `boot::spawn_installed_servers` during startup.
- `src/openhuman/tool_registry/ops.rs` — uses `connections` to surface MCP tools to agents.
- `src/openhuman/tools/ops.rs` + `src/openhuman/tools/impl/network/mcp_setup.rs` — agent tool wrappers.
- `src/openhuman/about_app/catalog.rs` — capability entry `channels.mcp_registry_browse`.
- `src/bin/test_mcp_stub.rs` + `tests/mcp_registry_e2e.rs` — E2E stub server.

## Notes / gotchas

- **`mcp_clients` vs `mcp_registry`**: the RPC namespace and db filename keep the old `mcp_clients` name for backwards-compat; only the Rust module path is `mcp_registry`.
- **Two install paths**: legacy `mcp_clients_install` is **stdio-only** (filters for `stdio` connections). HTTP-remote installs go exclusively through the newer `mcp_setup_install_and_connect`, which routes via `pick_connection` (preference: published stdio → any stdio → published http_remote → any http_remote).
- **Smithery DTO naming**: the canonical result shapes are named `Smithery*` for wire compat; non-Smithery registries adapt into the same shapes and tag the `source` field.
- **HTTP-remote env**: env vars for HTTP-remote installs (typically OAuth tokens) are picked up by `McpHttpClient`'s own auth config, not injected at dial time by this module.
- **Secret safety**: raw secret values flow only through `submit_secret` and the just-in-time resolve in `test_connection`/`install_and_connect`; never echoed in responses or logged. `consume_refs` only removes refs after values are persisted.
- **Boot is best-effort**: a misbehaving server logs and is skipped; it never blocks core startup.
- **`all_connected_tools` caveat**: it currently returns `server_id` in the `qualified_name` slot — callers needing the real qualified name must re-join against `store::list_servers`.
- **Official registry cursor walk**: deep-page cache misses walk pages sequentially up to `MAX_CURSOR_WALK_PAGES` (50) before bailing to avoid request amplification.
