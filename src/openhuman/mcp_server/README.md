# mcp_server

Opt-in **Model Context Protocol (MCP) server** that exposes a curated, security-gated slice of OpenHuman's tool surface (memory-tree reads/writes, core/agent introspection, subagent execution, SearXNG search) and bundled prompt assets to external MCP clients (Claude Desktop, Cursor, Windsurf, …). Started via `openhuman-core mcp` — stdio transport by default, or `--transport http` for Streamable HTTP + SSE on a local bind address. It is a JSON-RPC dispatcher, not a registered RPC domain: it has no `schemas.rs`/controllers and is wired only through `src/core/cli.rs`, translating each MCP `tools/call` into an existing registered core RPC method.

## Responsibilities

- Implement the MCP JSON-RPC server lifecycle: `initialize`, `ping`, `tools/list`, `tools/call`, `resources/list`, `resources/templates/list`, `resources/read`, plus notifications (`notifications/initialized`, `notifications/cancelled`).
- Advertise a fixed catalog of MCP tools (`tool_specs`) with input JSON-schemas and MCP `ToolAnnotations` (`readOnlyHint`/`destructiveHint`/`idempotentHint`/`openWorldHint`).
- Validate/normalize tool arguments at the MCP layer (explicit rejection over silent clamping), map them to registered core RPC params, and dispatch via `all::try_invoke_registered_rpc`.
- Enforce `SecurityPolicy` per call: read tools require `ToolOperation::Read`; `agent.run_subagent` and the three write tools require `ToolOperation::Act`.
- Run write tools (`memory.store`, `memory.note`, `tree.tag`) through a dedicated write-dispatch + audit pipeline that records every attempt (success and rejection) to the MCP write-audit log.
- Serve bundled prompt assets (`IDENTITY.md`, `SOUL.md`, `USER.md`, and each built-in subagent's `prompt.md`) as static MCP resources under the `openhuman://prompts/...` URI scheme.
- Provide two transports — newline-delimited JSON-RPC over stdio, and Axum-based Streamable HTTP + SSE with session-id + protocol-version handshakes and optional bearer auth.
- Capture client provenance from `initialize` `clientInfo.name` into a per-session `source_type` (e.g. `mcp:claude-desktop`) used for audit attribution.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/mcp_server/mod.rs` | Module docstring + private submodule decls; re-exports `run_http`/`HttpServerConfig`, `run_stdio_from_cli`, `tool_specs`/`McpToolSpec`. |
| `src/openhuman/mcp_server/protocol.rs` | JSON-RPC 2.0 dispatch core: parses lines/values (single + batch), routes `initialize`/`ping`/`tools/*`/`resources/*`, builds success/error envelopes, negotiates protocol version. |
| `src/openhuman/mcp_server/tools.rs` | Tool catalog (`tool_specs`, `base_tool_specs`, `searxng_tool_spec`), input schemas, argument validation, `call_tool` dispatch, read/act policy enforcement, subagent execution, slug/key helpers. |
| `src/openhuman/mcp_server/write_dispatch.rs` | Write/audit pipeline for `memory.store`/`memory.note`/`tree.tag`: config load, act-policy enforcement, RPC dispatch to `openhuman.memory_doc_put`, audit-record write (success/rejection), PII-redacting arg summaries. |
| `src/openhuman/mcp_server/resources.rs` | Static `RESOURCE_CATALOG` of compile-time-embedded (`include_str!`) prompt markdown; `resources/list`, `resources/templates/list` (always empty), `resources/read`. Test cross-checks catalog vs `agent::agents::BUILTINS`. |
| `src/openhuman/mcp_server/session.rs` | `McpSession` — captures + normalizes client name from `initialize` into a `source_type`; first observation locks the value. |
| `src/openhuman/mcp_server/http.rs` | Axum Streamable HTTP + SSE transport (`run_http`, `HttpServerConfig`): POST/GET/DELETE on `/`, session map, protocol-version checks, optional `Authorization: Bearer`, SSE keep-alive, session-id redaction. |
| `src/openhuman/mcp_server/stdio.rs` | CLI entry `run_stdio_from_cli` (arg parse: `--transport`/`--host`/`--port`/`--auth-token`/`-v`/`--help`), logging init, stdio read/write loop (`run_stdio`). |
| `src/openhuman/mcp_server/tools_tests.rs` | Sibling `#[cfg(test)]` suite for `tools.rs` (via `#[path]`). |

## Public surface

Re-exported from `mod.rs`:

- `run_stdio_from_cli(args: &[String]) -> Result<()>` — CLI entry point; builds its own tokio runtime and selects stdio vs HTTP transport.
- `run_http(config: HttpServerConfig) -> Result<()>` and `HttpServerConfig { bind_addr, auth_token }` — HTTP/SSE server.
- `tool_specs() -> Vec<McpToolSpec>` and `McpToolSpec { name, title, description, rpc_method, input_schema, annotations }` — the advertised tool catalog.

## RPC / controllers

This module exposes **no** registered core RPC methods (no `schemas.rs`, no controllers, no `openhuman.mcp_server_*` namespace). It is the **client/server-of-MCP**, not an RPC domain. Instead it *consumes* existing registered RPC methods, mapping each MCP tool to one via `all::try_invoke_registered_rpc` after validating against `all::schema_for_rpc_method`:

| MCP tool | Mapped core RPC method |
| --- | --- |
| `memory.search` | `openhuman.memory_tree_search` |
| `memory.recall` | `openhuman.memory_tree_recall` |
| `tree.read_chunk` | `openhuman.memory_tree_get_chunk` |
| `tree.browse` | `openhuman.memory_tree_list_chunks` |
| `tree.top_entities` | `openhuman.memory_tree_top_entities` |
| `tree.list_sources` | `openhuman.memory_tree_list_sources` |
| `memory.store` / `memory.note` / `tree.tag` | `openhuman.memory_doc_put` |
| `searxng_search` | `openhuman.tools_searxng_search` |
| `core.list_tools` / `core.tool_instructions` / `agent.list_subagents` / `agent.run_subagent` | (no RPC mapping — handled in-process via `Agent` / `AgentDefinitionRegistry`) |

## Agent tools

It does **not** own any agent tools in the `tools.rs`/`src/openhuman/tools` sense. The "tools" here are **MCP-protocol tools** advertised to external clients:

- Read-only (`ToolOperation::Read`): `core.list_tools`, `core.tool_instructions`, `agent.list_subagents`, `memory.search`, `memory.recall`, `tree.read_chunk`, `tree.browse`, `tree.top_entities`, `tree.list_sources`, `searxng_search` (config-gated on `searxng.enabled`).
- Act-policy (`ToolOperation::Act`): `agent.run_subagent` (annotated destructive/open-world; rejects `integrations_agent`), and the write tools `memory.store`, `memory.note`, `tree.tag` (annotated destructive/idempotent, local-only).

Argument bounds enforced in-layer: `k`/limits capped at `MAX_LIMIT` (50), default 10; `tree.tag` capped at 50 tags / 128 bytes per tag; SearXNG `max_results` capped at `SEARXNG_MAX_RESULTS`. Write tools derive deterministic upsert keys (`mcp-store-<slug>`, `mcp-note-<chunk_id>`, `mcp-tag-<chunk_id>`).

## Events

None. This module publishes/subscribes no `DomainEvent`s and has no `bus.rs`.

## Persistence

No `store.rs`. The only durable side effect is the **MCP write-audit log**, written via `crate::openhuman::mcp_audit::record_write` (separate `mcp_audit` domain) for every write-tool attempt — including pre-dispatch rejections. Audit rows store a PII-redacted `args_summary` (titles truncated to 128 chars; `content`/`note_text` bodies omitted, only lengths/counts kept), `client_info`, success flag, and resulting `document_id`. Audit inserts run off the hot path via `spawn_blocking` (or a thread fallback). HTTP session records (`Mcp-Session-Id` → negotiated protocol version) are kept in an in-memory map only.

## Dependencies

- `crate::core::all` — `try_invoke_registered_rpc`, `schema_for_rpc_method`, `validate_params`: dispatch MCP tool calls into the registered core RPC layer and validate params against controller schemas.
- `crate::core::logging` (`CliLogDefault`, `init_for_cli_run`) — install the stderr tracing subscriber for the MCP subprocess.
- `crate::openhuman::config` (`Config`, `rpc::load_config_with_timeout`, `McpAuthConfig`/`McpClientIdentityConfig` in tests) — load config for policy/searxng gating and per-call config.
- `crate::openhuman::security` (`SecurityPolicy`, `ToolOperation`) — enforce read/act autonomy policy per tool call.
- `crate::openhuman::agent` (`Agent`, `agents::BUILTINS`, `harness::AgentDefinitionRegistry`) — build the orchestrator agent for `core.list_tools`/`core.tool_instructions`, list/run subagents, and cross-check the resource catalog.
- `crate::openhuman::inference::provider::traits::build_tool_instructions_text` — render the markdown tool-use instructions block for `core.tool_instructions`.
- `crate::openhuman::tools` (`SEARXNG_MAX_RESULTS`, `normalize_categories`) — SearXNG bounds + category normalization for `searxng_search`.
- `crate::openhuman::mcp_audit` (`record_write`, `NewMcpWriteRecord`, list/query helpers in tests) — durable write-audit log.
- `crate::openhuman::mcp_client::McpHttpClient` — round-trip test harness for the HTTP transport (test-only).
- External crates: `axum`/`tokio`/`tokio-stream` (HTTP+SSE), `serde_json`, `uuid`, `sha2`/`hex` (session-id redaction, slug fallback hash), `chrono` (audit timestamps).

## Used by

- `src/core/cli.rs` — dispatches `mcp` / `mcp-server` subcommands to `run_stdio_from_cli`; the only production entry point. (`src/core/legacy_aliases.rs` references the command surface; `about_app/catalog.rs` lists it in the capability catalog.)
- Other `mcp_*` modules (`mcp_registry`, `mcp_client`, `mcp_audit`, `tool_registry`) are **siblings** in the broader MCP feature set, not consumers of this server's code paths (except the test-only `McpHttpClient` round-trip).

## Notes / gotchas

- **Not a controller-registry domain.** Don't look for `schemas.rs`/`all_controller_schemas` — exposure is via the CLI only, and tool calls re-enter the registry through `core::all`.
- **`ToolCallError` variant → JSON-RPC code is deliberate:** `InvalidParams` → `-32602` (client-actionable, including policy denials so the reason text surfaces), `Internal` → `-32603` (config load / platform failures). Policy denials are intentionally `InvalidParams`, not `Internal`.
- **Explicit rejection over silent clamping** throughout: over-cap `k`, oversize/over-count tags, blank required strings, and unexpected arguments all error rather than being trimmed/dropped.
- **Write audit is fire-and-forget but mandatory:** rejections are audited even before config is loaded (`audit_write_rejection_without_config`), and `dispatch_write_tool` returns `Ok(tool_error(...))` (not `Err`) on RPC-handler failure so the client gets an MCP `isError` result while the failure is still recorded.
- **Protocol negotiation:** supports `2024-11-05`, `2025-03-26`, `2025-06-18`, and `2025-11-25` (`LATEST_PROTOCOL_VERSION`); unknown requested versions fall back to latest. HTTP enforces an exact session protocol-version match on subsequent requests.
- **HTTP security:** bearer auth is optional (`--auth-token`); session ids are SHA-256-redacted in logs; default bind is `127.0.0.1:9300`.
- **Resource catalog parity is CI-enforced:** `resources.rs` content is `include_str!`-embedded at compile time and the `catalog_mirrors_builtins` test fails if a built-in subagent lacks a matching `openhuman://prompts/agents/<id>` entry.
- **`agent.run_subagent` limits:** rejects `integrations_agent` (toolkit binding not yet supported over first-level MCP) and runs a fresh single-turn agent session tagged with an `mcp:<agent_id>:<uuid>` event context.
- **stdio logging defaults to `warn`** on stderr (so failures surface in client UIs); `--verbose` → `debug`; a user-set `RUST_LOG` always wins. Stdout is reserved for protocol messages only.
