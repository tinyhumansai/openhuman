# mcp/registry — user-installed MCP servers

Host half of the dynamic, user-installed MCP server surface. The registry
itself — the Smithery and official catalogs, the SQLite store, the live
connection map, the subprocess/browser-sign-in supervisor, and the setup
secret vault — moved to [`tinymcp`](https://github.com/tinyhumansai/tinymcp).
What is left here is what belongs to this application: the `mcp_clients` and
`mcp_setup` RPC surface, the agent-facing tools, the prompt-injection scan
over remote tool definitions, and turning what the reconnect supervisor
observed into this application's own events.

The RPC namespace and the on-disk database filename are still `mcp_clients`,
unchanged across the move — existing frontend code and existing on-disk state
keep working. The Rust module path is `crate::mcp::registry`.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Module declarations, the `connections`/`store`/`boot`/`supervisor`/`oauth` re-export facades, and `tools_safe_for_agent` (the prompt-injection scan). |
| `ops.rs` / `ops_tests.rs` | `mcp_clients_*` RPC handler bodies — each delegates to the service `mcp::host` holds. |
| `setup_ops.rs` / `setup_ops_tests.rs` | `mcp_setup_*` guided-setup handler bodies. |
| `schemas.rs`, `schemas/` (`mod.rs`, `registry.rs`, `handlers.rs`, `params.rs`, `setup_registry.rs`, `setup_handlers.rs`), `schemas_tests.rs` | Controller schema registry and dispatch. |
| `supervisor_events.rs` / `supervisor_events_tests.rs` | Maps a supervisor tick's `TickReport` into `DomainEvent`s. |
| `bus.rs` / `bus_tests.rs` | `McpClientEventSubscriber` — logs lifecycle events for observability. |
| `tools.rs` / `tools_tests.rs` | Agent-facing `mcp_registry_*` tools, thin shims over `ops.rs`. |
| `helpers.rs` | Shared identifier validation, workspace-service resolution, and env-key injection used by both `ops.rs` and `setup_ops.rs`. |
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
`registry_get`, `installed_list`, `install`, `update_env`, `uninstall`,
`detect_auth`, `oauth_begin`, `connect`, `disconnect`, `status`, `tool_call`,
`config_assist`, `registry_settings_get`, `registry_settings_set`,
`set_enabled`. `setup_ops.rs` implements the `mcp_setup` namespace: `search`,
`get`, `request_secret`, `submit_secret`, `test_connection`,
`install_and_connect`. Both are registered together by
`schemas::all_registered_controllers` (`schemas/registry.rs`).

What stayed host-side inside these handlers, on purpose:

- **Events** — `tinymcp` reports outcomes in its return values and publishes
  nothing; `ops.rs`/`setup_ops.rs` turn those into `DomainEvent`s because the
  vocabulary is this application's.
- **The prompt-injection scan** — `tools_safe_for_agent` filters remote tool
  definitions before they reach the agent.
- **The configuration-assistant agent turn** — `tinymcp` gathers catalog
  detail and the credential names an install would need; running the model
  turn needs the agent, the tool surface, and the approval gate, all of which
  live in this crate.
- **Setup secret handles** — the `secret://…` opaque handle flow: `tinymcp`
  owns the vault, this layer publishes the event that prompts the user
  out-of-band and waits for the answer. The raw value never crosses the
  model-facing surface.

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
`mcp_registry_config_assist`, `mcp_registry_install`,
`mcp_registry_uninstall` — thin shims over `ops.rs`. Discovery/observe/
connect/call tools are default-ON; `install`/`uninstall` (persistent writes
of installed state and secrets) ship default-OFF, gated behind the
`mcp_manage` toggle in `tools/user_filter.rs`. Re-exported by
`tools/mod.rs` behind `#[cfg(feature = "mcp")]`. The `mcp_setup_*`
setup-agent tools and the generic `mcp_list_servers`/`mcp_call_tool` bridge
tools live elsewhere and are a distinct surface from these
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
- `crates/openhuman-core/src/core/jsonrpc.rs` — the `/oauth/mcp/callback`
  route calls `oauth::complete`.
- `crates/openhuman-core/src/tools/impl/network/mcp_setup.rs` — the
  `mcp_setup_*` agent tools wrap `setup_ops`.
- `crates/openhuman-core/src/tools/registry/ops.rs` and
  `crates/openhuman-core/src/agent/registry/agents/orchestrator/prompt.rs` —
  read `mcp::registry::connections` to list connected servers/tools for the
  tool catalog and the orchestrator prompt.
- `crates/openhuman-core/src/agent/session_host/turn/core_turn.rs` — reads
  `connections::connected_overview()` when assembling a turn.
- `crates/openhuman-core/src/platform/about_app/catalog_auth_channels_team.rs` — the
  `channels.mcp_registry_browse` / `mcp_server_install` /
  `mcp_server_connect` capability entries.
