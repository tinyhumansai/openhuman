# tool_registry

Unified **read-only** tool registry for OpenHuman. It builds a single discovery view across every tool surface the agent can call — MCP stdio server tools, JSON-RPC controller-backed tools, and tools from currently-connected MCP client servers — and exposes them with stable ids, normalized JSON Schemas, transport/route metadata, tags, and health. It also produces redacted policy/tool-visibility diagnostics (autonomy posture, MCP allowlists, MCP write-audit health, recent policy denials, capability-provider summaries) and tracks an in-memory ring buffer of recent agent-tool policy denials. Nothing here executes tools; it only enumerates and describes them.

## Responsibilities

- Build a sorted, de-duplicated registry snapshot from three sources: `mcp_server` stdio tool specs, `tools` controller schemas, and `mcp_registry` connected-client tools.
- Normalize controller `ControllerSchema` inputs/outputs into JSON Schema; attach transport (`json_rpc` / `mcp_stdio`), route metadata, tags, `allowed_agents` (`*` in this MVP), `enabled`, and `health`.
- Serve `list` / `get` (by `tool_id`) RPC lookups over the registry.
- Produce redacted `diagnostics`: tool counts by transport, heuristic write-capable surfaces, policy surfaces, autonomy posture, MCP allowlist summaries, MCP write-audit row counts (last 24h), recent denials, and capability-provider counts.
- Maintain a bounded, secret-redacting in-memory log of recent policy denials (`denials.rs`).
- Normalize and validate configured external **capability providers** (id slugging, dedupe, trust/enabled state) for policy and diagnostics callers (`providers.rs`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/tool_registry/mod.rs` | Export-focused. Re-exports ops, providers, schemas (as `all_tool_registry_*`), and types. |
| `src/openhuman/tool_registry/ops.rs` | Core logic: `registry_entries()`, `list_tools()`, `get_tool()`, `diagnostics()` / `diagnostics_for_config()`, plus schema→JSON-Schema conversion, tagging, write-capability heuristics, MCP write-audit health query. |
| `src/openhuman/tool_registry/types.rs` | Serde response types: `ToolRegistryEntry`, `ToolRegistryList`, `ToolRegistryTransport`, `ToolRegistryHealth`, `ToolPolicyDiagnostics` + sub-structs, `RecentPolicyDenial`, `CapabilityProviderDiagnostics`. |
| `src/openhuman/tool_registry/schemas.rs` | Controller schemas + `handle_list` / `handle_get` / `handle_diagnostics` handlers delegating to `ops.rs`. Inline tests. |
| `src/openhuman/tool_registry/providers.rs` | `CapabilityProviderRegistry` over config: id normalization, dedupe, trust checks, redacted diagnostics. Inline tests. |
| `src/openhuman/tool_registry/denials.rs` | Static `Mutex<VecDeque>` ring buffer (max 50) of recent policy denials; `record()` / `list()`; redacts secret markers, truncates reasons. Inline tests. |
| `src/openhuman/tool_registry/ops_tests.rs` | Sibling test module for `ops.rs` (`#[path]`-included). |

## Public surface

- From `ops`: `get_tool`, `list_tools`, `registry_entries`.
- From `providers`: `CapabilityProviderMetadata`, `CapabilityProviderRegistry`, `CapabilityProviderRegistryError`, `capability_provider_by_id`, `capability_provider_diagnostics`, `capability_provider_registry`, `is_capability_provider_trusted_enabled`, `list_capability_providers`, `normalize_capability_provider_id`.
- From `schemas`: `all_tool_registry_controller_schemas`, `all_tool_registry_registered_controllers`.
- From `types`: `ToolRegistryEntry`, `ToolRegistryList`, `ToolRegistryTransport`, `ToolRegistryHealth`, `ToolPolicyDiagnostics`, `ToolPolicyPosture`, `McpAllowlistDiagnostics`, `McpServerAllowlistSummary`, `McpWriteAuditHealth`, `RecentPolicyDenial`, `CapabilityProviderDiagnostics`.
- `denials` is `pub mod` — consumers call `tool_registry::denials::record(...)` directly.

## RPC / controllers

Namespace `tool_registry`, registered via `all_tool_registry_registered_controllers` (wired in `src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |
| `tool_registry.list` (`openhuman.tool_registry_list`) | none | `tools`: array of registry entries |
| `tool_registry.get` | `tool_id` (required string) | `tool`: one registry entry |
| `tool_registry.diagnostics` | none | `diagnostics`: redacted counts/posture/allowlists/denials/providers |

All handlers return `RpcOutcome<T>` serialized via `into_cli_compatible_json()`.

## Persistence

No owned persistence. `diagnostics()` reads (read-only) the `mcp_writes` table via `memory_store::chunks::store::with_connection` to count rows in the last 24h for `McpWriteAuditHealth`. Recent denials live in a **process-global in-memory** `Mutex<VecDeque>` in `denials.rs` (not durable; max 50 entries).

## Dependencies

- `crate::core::all` — `all_controller_schemas()` / `rpc_method_name()` to enumerate controller tools and resolve their RPC method names for routes/policy surfaces.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` and `core::all::{ControllerFuture, RegisteredController}` — schema model and controller registration contract.
- `crate::openhuman::config` (`Config`, `config::schema::CapabilityProviderTrustState`) — autonomy posture, MCP client allowlists, capability-provider config.
- `crate::openhuman::mcp_server` (`McpToolSpec`, `tool_specs()`) — MCP stdio tool source for registry entries.
- `crate::openhuman::mcp_registry::connections` (`all_connected_tools()`) — live MCP client server tools, fetched via `block_in_place` only on the multi-thread runtime.
- `crate::openhuman::memory_store::chunks::store` — read-only `mcp_writes` audit query.
- `crate::rpc::RpcOutcome` — RPC result envelope.

## Used by

- `src/core/all.rs` — registers controllers/schemas and routes the `tool_registry` namespace.
- `src/openhuman/agent/harness/session/turn.rs` — calls `tool_registry::denials::record(...)` to log agent-tool policy denials.
- `src/openhuman/about_app/catalog.rs` — capability catalog references the registry surface.
- `src/openhuman/mcp_registry/connections.rs` — provides `all_connected_tools()` for registry integration.

## Notes / gotchas

- **Read-only by design** — the registry never executes tools; routes are descriptive metadata only.
- **Duplicate `tool_id` is first-write-wins**: ordered MCP-stdio → controller → MCP-client; duplicates (e.g. external servers reusing well-known names) are logged and skipped, not overwritten.
- **MCP client enumeration is best-effort**: on a single-thread tokio runtime (e.g. unit tests) or with no runtime, connected-client tools silently fall back to empty, since `block_in_place` panics outside the multi-thread runtime.
- `looks_write_capable` is a **heuristic** over tool-id keywords (add/create/delete/send/write/…), surfaced as `possible_write_surfaces` for review — not an authoritative permission check.
- `policy_surfaces` includes a fixed seed list plus any controller whose method starts with `security.` / `approval.`.
- Denial reasons are redacted on the markers `Bearer `, `sk-`, `ghp_`, `-----BEGIN` and truncated to 240 chars; entries are bounded to 50.
- Capability-provider ids are slugged to lowercase alphanumerics with `-`/`_`/`.` separators (max 96 chars); invalid or post-normalization-duplicate ids return `CapabilityProviderRegistryError`.
- `version` on every entry is the core crate version (`CARGO_PKG_VERSION`), used as the registry schema/version marker.
