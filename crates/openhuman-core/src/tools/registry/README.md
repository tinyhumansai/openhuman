# tool_registry

Unified, read-only tool registry for OpenHuman. It builds a single discovery view across every tool surface the agent can call: MCP stdio server tools, JSON-RPC controller-backed tools, and tools from currently-connected MCP client servers, and exposes them with stable ids, normalized JSON Schemas, transport/route metadata, tags, and health. It also produces redacted policy/tool-visibility diagnostics (autonomy posture, MCP allowlists, MCP write-audit health, recent policy denials, capability-provider summaries) and tracks an in-memory ring buffer of recent agent-tool policy denials. Nothing here executes tools; it only enumerates and describes them.

## Responsibilities

- Build a sorted, de-duplicated registry snapshot from three sources: `mcp::server` stdio tool specs, `tools` controller schemas (`crate::tools::all_tools_controller_schemas()`), and `mcp::registry::connections` connected-client tools.
- Normalize controller `ControllerSchema` inputs/outputs into JSON Schema; attach transport (`json_rpc` / `mcp_stdio`), route metadata, tags, `allowed_agents` (`*` in this MVP), `enabled`, and `health`.
- Serve `list` / `get` (by `tool_id`) RPC lookups over the registry.
- Produce redacted `diagnostics`: tool counts by transport, heuristic write-capable surfaces, policy surfaces, autonomy posture, MCP allowlist summaries, MCP write-audit row counts (last 24h), the 25 most recent denials, and capability-provider counts.
- Maintain a bounded, secret-redacting in-memory log of recent policy denials (`tinytools::registry::denials`).
- Normalize and validate configured external capability providers (id slugging, dedupe, trust/enabled state) for policy and diagnostics callers ([`providers.rs`](./providers.rs)).

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/tools/registry/mod.rs` | Export-focused. `pub mod ops`; re-exports ops entry points, providers, schemas (as `all_tool_registry_*`), and types. |
| `crates/openhuman-core/src/tools/registry/ops.rs` | Core logic: `registry_entries()` / `registry_entries_for_config()`, `list_tools()`, `get_tool()`, `diagnostics()` / `diagnostics_for_config()`, plus schema→JSON-Schema conversion, tagging, write-capability heuristics, MCP write-audit health query. |
| `crates/openhuman-core/src/tools/registry/schemas.rs` | Controller schemas + `handle_list` / `handle_get` / `handle_diagnostics` handlers delegating to [`ops.rs`](./ops.rs). |
| `crates/openhuman-core/src/tools/registry/providers.rs` | `CapabilityProviderRegistry` over config: id normalization, dedupe, trust checks, redacted diagnostics. |
| `crates/openhuman-core/src/tools/registry/{ops,schemas,providers}_tests.rs` | Sibling `#[path]`-included test modules for each file above. |

## Public surface

- From `ops`: `get_tool`, `list_tools`, `registry_entries`, `registry_entries_for_config`. `ops` is also `pub mod`, so `ops::diagnostics_for_config` is reachable (used by the raw-coverage tests).
- From `providers`: `CapabilityProviderMetadata`, `CapabilityProviderRegistry`, `CapabilityProviderRegistryError`, `capability_provider_by_id`, `capability_provider_diagnostics`, `capability_provider_registry`, `is_capability_provider_trusted_enabled`, `list_capability_providers`, `normalize_capability_provider_id`.
- From `schemas`: `all_tool_registry_controller_schemas`, `all_tool_registry_registered_controllers`.
- From `types`: `ToolRegistryEntry`, `ToolRegistryList`, `ToolRegistryTransport`, `ToolRegistryHealth`, `ToolPolicyDiagnostics`, `ToolPolicyPosture`, `McpAllowlistDiagnostics`, `McpServerAllowlistSummary`, `McpWriteAuditHealth`, `RecentPolicyDenial`, `CapabilityProviderDiagnostics`.
- Policy vocabulary and the denial ring buffer live in `tinytools::registry`; the host only re-exports the types.

## RPC / controllers

Namespace `tool_registry`, registered via `all_tool_registry_registered_controllers` (wired in `crates/openhuman-core/src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |
| `tool_registry.list` (`openhuman.tool_registry_list`) | none | `tools`: array of registry entries |
| `tool_registry.get` (`openhuman.tool_registry_get`) | `tool_id` (required string) | `tool`: one registry entry |
| `tool_registry.diagnostics` (`openhuman.tool_registry_diagnostics`) | none | `diagnostics`: redacted counts/posture/allowlists/denials/providers |

All handlers return `Outcome<T>` serialized via `into_cli_compatible_json()`.

## Persistence

No owned persistence. `diagnostics()` reads the MCP write-audit log through `crate::mcp::audit::list_writes` (with a `McpWriteListQuery` whose `since_ms` is now minus 24h, capped at `tinymcp_bus::MAX_LIST_LIMIT` rows) to fill `McpWriteAuditHealth`; a query error lands in `last_error` rather than failing diagnostics. Recent denials live in a process-global, in-memory `Mutex<VecDeque>` in `tinytools::registry::denials` (not durable; max 50 entries).

## Dependencies

- `crate::core::all`: `rpc_method_name()` for controller route metadata and `all_controller_schemas()` for the `security.` / `approval.` policy-surface scan. Registry *entries* for controllers come from `crate::tools::all_tools_controller_schemas()`, not the whole controller set.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` and `core::all::{ControllerFuture, RegisteredController}`: schema model and controller registration contract.
- `crate::config` (`Config`, `config::schema::CapabilityProviderTrustState`): autonomy posture, MCP client allowlists, capability-provider config.
- `crate::mcp::server` (`McpToolSpec`, `tool_specs()`): MCP stdio tool source for registry entries.
- `crate::mcp::registry::connections` (`all_connected_tools()` / `all_connected_tools_for_config()`, defined inside `mcp/registry/mod.rs`; empty-returning stubs in `mcp/registry/stub.rs` when the `mcp` feature is disabled): live MCP client server tools, fetched via `block_in_place` only on the multi-thread runtime.
- `crate::mcp::audit` (`list_writes`, `McpWriteListQuery`; stubbed under `mcp/audit/stub.rs` without the `mcp` feature): write-audit health.
- `crate::core::Outcome`: RPC result envelope.

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers controllers/schemas and routes the `tool_registry` namespace.
- `crates/openhuman-core/src/agent/tinyagents/middleware/tool_policy.rs`: calls `tinytools::registry::denials::record(...)` when a tool call is denied or requires approval.
- `crates/openhuman-core/src/platform/about_app/catalog_conversation_intelligence.rs`: the `intelligence.tool_registry` capability entry points users at `openhuman.tool_registry_list` / `openhuman.tool_registry_get`.
- `tests/raw_coverage/tool_registry_approval_e2e.rs` and siblings: exercise `denials`, `ops::diagnostics_for_config`, and the provider registry directly.

## Notes / gotchas

- Read-only by design: the registry never executes tools; routes are descriptive metadata only.
- Duplicate `tool_id` is first-write-wins: ordered MCP-stdio → controller → MCP-client; duplicates (e.g. external servers reusing well-known names) are logged and skipped, not overwritten. MCP-client ids are `mcp-client::<server_id>::<tool>` and route to `openhuman.mcp_clients_tool_call`.
- MCP client enumeration is best-effort: on a single-thread tokio runtime (e.g. unit tests) or with no runtime, connected-client tools silently fall back to empty, since `block_in_place` panics outside the multi-thread runtime.
- `registry_entries()` vs `registry_entries_for_config()`: the ambient form resolves the MCP host through the process default, which stops answering once a second workspace is open in the process (test binaries). Callers holding a `Config` should use the `_for_config` variant.
- `looks_write_capable` is a heuristic over tool-id keywords (add/create/delete/send/write/etc.), surfaced as `possible_write_surfaces` for review. It is not an authoritative permission check.
- `policy_surfaces` includes a fixed seed list plus any controller whose method starts with `security.` / `approval.`.
- Denial reasons containing any of `Bearer `, `sk-`, `ghp_`, `-----BEGIN` are replaced wholesale with `[redacted: sensitive content]`, then truncated to 240 chars; entries are bounded to 50, and blank tool names are dropped.
- Capability-provider ids are slugged to lowercase alphanumerics with `-`/`_`/`.` separators (max 96 chars); invalid or post-normalization-duplicate ids return `CapabilityProviderRegistryError`.
- `version` on every entry is the core crate version (`CARGO_PKG_VERSION`), used as the registry schema/version marker.

## Further reading

- [Parent module (`tools`)](../README.md)
- [Native tools overview](../../../../../docs/gitbooks/en/features/native-tools/README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [tinyagents submodule](../../../../../vendor/tinyagents/README.md)
