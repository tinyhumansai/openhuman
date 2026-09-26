# mcp/registry — user-declared MCP servers

Host half of the dynamic, user-declared MCP server surface. The registry
itself — the Smithery and official catalogs, the SQLite store, the live
connection map, and the subprocess/browser-sign-in supervisor — moved to
[`tinymcp`](https://github.com/tinyhumansai/tinymcp). What is left here is
what belongs to this application: the `mcp_clients` RPC surface, the
`mcp.json` document that is the only way a server is added or removed, the
agent-facing tools, the prompt-injection scan over remote tool definitions,
and turning what the reconnect supervisor observed into this application's
own events.

The catalogs are **browse-only**. There is no install-from-catalog RPC, no
install tool and no setup agent: a user finds a server in the Registry tab,
opens its own page, and declares it in `mcp.json`.

The RPC namespace and the on-disk database filename are still `mcp_clients`,
unchanged across the move — existing frontend code and existing on-disk state
keep working. The Rust module path is `crate::mcp::registry`.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Module declarations, the `connections`/`store`/`boot`/`supervisor`/`oauth` re-export facades, and `tools_safe_for_agent` (the prompt-injection scan). |
| `ops.rs` / `ops_tests.rs` | `mcp_clients_*` RPC handler bodies — each delegates to the service `mcp::host` holds. |
| `config_doc.rs` / `config_doc_tests.rs` | The `mcp.json` contract: how the store renders as a document (credential names only), what a written one may say, and the reconciliation helpers. |
| `config_ops.rs` | `mcp_clients_config_get` / `config_set`: replace the store with what the document declares. |
| `schemas/` (`mod.rs`, `registry.rs`, `handlers.rs`, `params.rs`), `schemas_tests.rs` | Controller schema registry and dispatch. |
| `supervisor_events.rs` / `supervisor_events_tests.rs` | Maps a supervisor tick's `TickReport` into `DomainEvent`s. |
| `bus.rs` / `bus_tests.rs` | `McpClientEventSubscriber` — logs lifecycle events for observability. |
| `tools.rs` / `tools_tests.rs` | Agent-facing `mcp_registry_*` tools, thin shims over `ops.rs`. |
| `action_tool.rs` / `action_tool_tests.rs` | One deferred agent tool per action on a connected server. Stable names and sanitized schemas enter `tool_search`; execution checks the current connection and routes through `ops.rs`. |
| `helpers.rs` | Shared identifier validation, workspace-service resolution, and env-key injection used by the handlers. |
| `stub.rs` | The `mcp`-less mirror of the always-on surface: `all_mcp_registry_registered_controllers` (empty), `boot`, `bus`, `supervisor`, `oauth`, and the three `connections` lookups always-on callers name. |

## Modules re-exported over `tinymcp`

`mod.rs` defines several thin facade modules rather than fresh logic:

- `types` — the payload vocabulary, entirely re-exported from `tinymcp_bus`
  (`InstalledServer`, `McpTool`, `ConnStatus`, `Transport`, the Smithery
  catalog types, …). A parallel set of types here would mean a conversion at
  every call site that nothing checks.
- `connections` — a thin view over the live connection map the `mcp::host`
  service holds: `connected_overview[_for_config]`,
  `all_connected_tools[_for_config]`, `tools_for`/`server_tools_for_config`,
  `is_connected[_for_config]`, `auth_hint_for[_config]`, `connect`,
  `disconnect[_for_config]`, `last_error_for[_config]`. Every lookup answers
  "nothing" (empty list / `false` / `None`) when the workspace has no open
  host yet, rather than erroring; only `connect` returns an error in that
  case.
- `store` — the one direct reach into the registry's store that outlived the
  extraction: `set_cached`, used by an end-to-end test to seed the upstream
  response cache without a real catalog call.
- `boot` — `spawn_installed_servers`, connecting every enabled installed
  server at startup; never fails (a broken third-party server is logged and
  skipped).
- `supervisor` — `run()`, the reconnect-supervisor loop: one task per
  process, walking every workspace host the process has opened each tick.
- `oauth` — `complete`, finishing a browser sign-in from the redirect
  callback and reconnecting the server.

## RPC surface

`ops.rs` implements the `mcp_clients` namespace: `registry_search`,
`registry_get`, `installed_list`, `update_env`, `uninstall`, `detect_auth`,
`oauth_begin`, `connect`, `disconnect`, `status`, `tool_call`,
`registry_settings_get`, `registry_settings_set`, `set_enabled`.
`config_ops.rs` adds `config_get` and `config_set` — the `mcp.json`
document. All are registered by `schemas::all_registered_controllers`
(`schemas/registry.rs`).

`config_set` is a *replace*: a server absent from the document is
uninstalled, a new one is inserted as an `InstalledServer` built straight
from its declaration (`command`/`args` → stdio, `url` → HTTP-remote), and one
whose dial changed is rewritten under the same `server_id` so the row the
frontend and the connection map address stays the same row. Credentials are
the exception: `env` (stdio) and `headers` (HTTP) are write-only — a read
renders only `envKeys` and `authConfigured` — so an entry saved without a
credential block keeps what is stored, and a key set to `""` removes that one
value. Enabled servers that were added or rewritten are connected in the
background; the status poll reports the outcome.

What stayed host-side inside these handlers, on purpose:

- **Events** — `tinymcp` reports outcomes in its return values and publishes
  nothing; the handlers turn those into `DomainEvent`s because the vocabulary
  is this application's.
- **The prompt-injection scan** — `tools_safe_for_agent` filters remote tool
  definitions before they reach the agent.
- **The document** — `tinymcp` has no notion of `mcp.json`; the shape, the
  refusals and the reconciliation are this application's.

## Reconnect-supervisor events

`supervisor_events::domain_events_for` maps each `TickReport` from
`tinymcp::Supervisor::tick` into `DomainEvent`s in the `mcp_client` domain:
`McpServerProbeTimedOut`, `TransportDropped`, `Reconnected`,
`ReconnectFailed`, `Parked`. Every event is stamped with the workspace whose
host was ticked, because one process supervises every workspace it has
opened over its life and a subscriber persisting an event (the notification
bridge) needs to file it under the right one. An answered probe — the
nominal case — is deliberately not an event; `tinymcp` logs it at trace
level. These events reach the developer Event Log (`GET /events/domain`) and,
for the stays-down/restored/parked cases, the desktop notification bridge.

## Agent tools

`tools.rs` exposes `mcp_registry_search`, `mcp_registry_get`,
`mcp_registry_installed_list`, `mcp_registry_status`,
`mcp_registry_list_tools`, `mcp_registry_connect`,
`mcp_registry_disconnect`, `mcp_registry_tool_call`,
`mcp_registry_uninstall` — thin shims over `ops.rs`. Discovery/observe/
connect/call tools are default-ON; `uninstall` (a persistent write of
installed state) ships default-OFF, gated behind the `mcp_manage` toggle in
`tools/user_filter.rs`. There is no install tool: servers are declared by
the user in `mcp.json`. Re-exported by `tools/mod.rs` behind
`#[cfg(feature = "mcp")]`. The generic `mcp_list_servers`/`mcp_call_tool`
bridge tools live elsewhere and are a distinct surface from these
`mcp_registry_*` tools.

## Compile-time gate (`mcp` feature)

Every member above except `types` is gated on the `mcp` feature; with it
off, `stub.rs` mirrors the always-on surface with empty/no-op bodies.
`types` stays ungated so `ConnectedServerOverview` and `McpTool` are the
same real type in both builds.

## Dependencies

- `tinymcp` / `tinymcp_bus` — the extracted registry library and its wire
  contract.
- `crate::mcp::host` — the per-workspace service holder every function here
  resolves through.
- `crate::core::bus` / `crate::core::events` — `DomainEvent` publishing.
- `crate::security::prompt_injection` — `scan_tool_definition`, used by
  `tools_safe_for_agent`.

## Used by

- `crates/openhuman-core/src/core/all.rs` (~line 444) — registers
  `all_mcp_registry_registered_controllers()`.
- `crates/openhuman-core/src/mcp/mod.rs` — `start`/`start_boot_jobs` wire up
  `bus::init()`, `boot::spawn_installed_servers`, and the reconnect
  supervisor.
- `tinymcp`'s live connection map is the process cache: startup connects
  installed servers, and connect, disconnect, config updates, and reconnects
  update the map. `connected_overview()` reads tool snapshots from that map;
  it does not call an MCP server on each chat turn.
- `crates/openhuman-core/src/core/jsonrpc.rs` — the `/oauth/mcp/callback`
  route calls `oauth::complete`.
- `crates/openhuman-core/src/tools/registry/ops.rs` and
  `crates/openhuman-core/src/agent/registry/agents/orchestrator/prompt.rs` —
  read `mcp::registry::connections` to list connected servers/tools for the
  tool catalog and the orchestrator prompt.
- `crates/openhuman-core/src/agent/session_host/turn/core_turn.rs` — reads
  `connections::connected_overview()` when assembling a turn.
- `crates/openhuman-core/src/platform/about_app/catalog_auth_channels_team.rs` — the
  `channels.mcp_registry_browse` / `mcp_server_install` /
  `mcp_server_connect` capability entries.
