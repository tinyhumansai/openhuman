# MCP registry host adapter

This module preserves OpenHuman's `mcp_clients` RPC and agent-tool surface
while delegating registry behavior to TinyMCP over `tinymcp-bus`. TinyMCP owns
the catalogs, installed-server store, connections, OAuth protocol, and reconnect
supervisor. OpenHuman owns product RPC schemas, configuration custody,
permission and exposure mapping, prompt-injection screening, and conversion of
module outcomes into domain events.

The RPC namespace and on-disk `mcp_clients` name remain stable. Servers are
added through the user's `mcp.json` document; there is no install-from-catalog
RPC or agent install tool.

## Key files

| File | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | Bus-backed registry facade, contract re-exports, and `tools_safe_for_agent` prompt-injection filter. |
| [`ops.rs`](./ops.rs) / [`ops_tests.rs`](./ops_tests.rs) | `mcp_clients_*` handlers and domain event mapping. |
| [`config_ops.rs`](./config_ops.rs) | `config_get` / `config_set` host adapter for the module-owned `mcp.json` document. |
| `schemas/` and [`schemas_tests.rs`](./schemas_tests.rs) | OpenHuman controller schemas and dispatch. |
| [`supervisor_events.rs`](./supervisor_events.rs) / tests | Converts module supervisor reports into OpenHuman domain events. |
| [`bus.rs`](./bus.rs) / tests | Logs lifecycle events for observability. |
| [`tools.rs`](./tools.rs) / tests | Maps module-provided registry tool specs to OpenHuman permissions and dispatch. |
| [`action_tool.rs`](./action_tool.rs) / tests | Builds deferred installed-server action tools and rechecks admission before execution. |
| [`stub.rs`](./stub.rs) | Empty/no-op registry facade when the `mcp` feature is disabled. |

## Behavior and boundaries

`ops.rs` and `config_ops.rs` call the configured TinyMCP object through
`mcp::module_client`. The host does not retain a second registry or connection
map. Calls that carry credentials use confidential TinyBus delivery; ordinary
calls must not be used for secrets. The module manages protocol state and
reports typed outcomes. The host translates those outcomes into OpenHuman's
RPC response shape and domain events.

`McpConfiguredClient` keeps each workspace's module object path private and
serializes its open, call, and close transitions. It retains transition
receipts across caller cancellation or lost replies so cleanup can be retried
without exposing TinyMCP's internal service implementation to host code.

`tools_safe_for_agent` is intentionally host-owned because it is the product's
prompt-injection policy. Remote descriptions and schemas are scrubbed by the
module before the host sees them; the host then applies this admission filter
before advertising a tool. Execution rechecks that the configured tool is
still present and safe.

`config_set` retains its replace semantics and existing credentials behavior:
environment values and HTTP headers remain write-only, while reads expose
environment key names and whether authentication is configured. The host
preserves user-facing policy and event behavior; document parsing, persistence,
reconciliation, and connection changes belong to TinyMCP.

The module's wire DTOs and operation names come from `tinymcp-bus`. Put protocol,
store, catalog, connection, OAuth, supervisor, or generic tool behavior in
`vendor/tinymcp`; update the OpenHuman submodule pin only to a published,
verified compatible release.

Run the host-focused suite with `cargo test -p openhuman --features modules,mcp
mcp::registry::`, and test the disabled feature build with
`cargo check -p openhuman --no-default-features`.
