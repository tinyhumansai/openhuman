# mcp/server

Opt-in **Model Context Protocol (MCP) server** that exposes a curated, security-gated slice of OpenHuman's tool surface (Memory v2 reads/writes, core/agent introspection, subagent execution, web search) and bundled prompt assets to external MCP clients (Claude Desktop, Cursor, Windsurf, …). Started via `openhuman-core mcp`: stdio transport by default, or `--transport http` for Streamable HTTP + SSE on a local bind address. It is a JSON-RPC dispatcher, not a registered RPC domain: it has no `schemas.rs`/controllers and is wired only through [`crates/openhuman-core/src/core/cli.rs`](../../core/cli.rs), translating each MCP `tools/call` into an existing registered core RPC method.

TinyMCP owns MCP framing, protocol state, callbacks, and HTTP admission through the `tinymcp_bus` server contract. OpenHuman's `module.rs` and `http.rs`/`stdio.rs` adapt the transports to that contract; `handler.rs` applies OpenHuman policy and dispatches the typed callbacks to the tool catalog, resources, and subagent-depth scope.

## Responsibilities

- Compose TinyMCP's protocol lifecycle for `initialize`, `ping`, tools, resources, prompts, and notifications with OpenHuman's host callbacks.
- Advertise a fixed catalog of MCP tools (`tool_specs`) with input JSON-schemas and MCP `ToolAnnotations` (`readOnlyHint`/`destructiveHint`/`idempotentHint`/`openWorldHint`).
- Map validated module callbacks to registered core RPC params and dispatch via `all::try_invoke_registered_rpc`.
- Enforce `SecurityPolicy` per call: read tools require `ToolOperation::Read`; `agent.run_subagent` and the two write tools (`memory.learn`, `memory.forget`) require `ToolOperation::Act`.
- Run write tools (`memory.learn`, `memory.forget`) through a dedicated write-dispatch + audit pipeline that records every attempt (success and rejection) to the MCP write-audit log.
- Serve bundled prompt assets and TinyFlows' two bus-owned agent archetypes as MCP resources under the `openhuman://prompts/...` URI scheme. Static markdown is embedded; flow prompts are fetched with the request's scoped config.
- Provide two transports: newline-delimited JSON-RPC over stdio, and Axum-based Streamable HTTP + SSE with session-id and protocol-version handshakes and optional bearer auth.
- Capture client provenance from `initialize` `clientInfo.name` into a per-session `source_type` (e.g. `mcp:claude-desktop`) used for audit attribution.

## Session lifetime and cleanup

Each host session serializes request handling and owns any pending host callback.
If a request is cancelled, the adapter cancels the matching module operation
and joins its callback before returning. Retries after a lost module reply
reuse the operation identity and completed callback reply.

For Streamable HTTP, the host asks TinyMCP to validate and apply DELETE first.
Only an accepted DELETE terminally closes the local session: the host rejects
new work, aborts and joins a running callback, then removes the session from
its table. A rejected DELETE leaves the session available for later requests.
Process shutdown closes the module sessions and drains their callbacks.

## Key files

| File | Role |
| --- | --- |
| [`crates/openhuman-core/src/mcp/server/mod.rs`](./mod.rs) | Module docstring + private submodule decls; re-exports `run_http`/`run_http_reporting`/`HttpServerConfig`, `run_stdio_from_cli`, `ensure_local_http`/`LocalMcpEndpoint`, `current_subagent_depth`/`HEADER_SUBAGENT_DEPTH`, `tool_specs`/`McpToolSpec`. |
| [`crates/openhuman-core/src/mcp/server/module.rs`](./module.rs) | Typed TinyBus adapter; submits frames, handles replayable callbacks, preserves host replies, and joins cancellation/close operations. |
| `crates/openhuman-core/src/mcp/server/handler.rs` | Applies host policy and dispatches typed `ServerHostCall`s to OpenHuman tool/resource behavior. |
| [`crates/openhuman-core/src/mcp/server/tools/`](./tools/) | Tool catalog and dispatch: `mod.rs` (facade; re-exports `tinymcp_bus::ToolCallError`), `types.rs` (`McpToolSpec` and the limit/tag constants, ungated so `McpToolSpec` is one real type in both builds), `specs.rs` (catalog builders and the `ServerToolSpec` conversion), `params.rs` (OpenHuman's per-tool argument policy → RPC params), `dispatch.rs` (`call_tool`/`list_tool_specs`, policy enforcement, subagent handlers). |
| [`crates/openhuman-core/src/mcp/server/write_dispatch.rs`](./write_dispatch.rs) | Write/audit pipeline for `memory.learn`/`memory.forget`: config load, act-policy enforcement, RPC dispatch to `openhuman.memory_learn` / `openhuman.memory_forget`, audit-record write (success/rejection) via `crate::mcp::audit::record_write`, PII-redacting arg summaries. |
| [`crates/openhuman-core/src/mcp/server/resources.rs`](./resources.rs) | Static prompt markdown catalog plus request-scoped TinyFlows Copilot `GetPrompt` and `GetResource` bus reads for the flow builtins. Tests cross-check exposed agent resources against `agent_registry::agents::BUILTINS`. |
| [`crates/openhuman-core/src/mcp/server/http.rs`](./http.rs) | Thin Axum transport adapter; passes raw request bodies and headers to module operations and uses module-selected response metadata. Gated on `all(feature = "mcp", feature = "http-server")`. |
| [`crates/openhuman-core/src/mcp/server/local.rs`](./local.rs) | Lazily-started, process-wide in-process loopback HTTP MCP server (`ensure_local_http`, `LocalMcpEndpoint`). Lets the sandboxed `claude` subprocess (Claude Code provider) reach OpenHuman's memory/tools over loopback without the MCP server inheriting Claude Code's OS jail; a per-process random bearer token stops any other local process from talking to it. |
| [`crates/openhuman-core/src/mcp/server/subagent_depth.rs`](./subagent_depth.rs) | Per-delegation-chain depth tracking for `agent.run_subagent`, propagated across the loopback MCP HTTP hop via the `X-OpenHuman-Subagent-Depth` header (read by `handler.rs` from `tinymcp`'s `RequestContext`) so nested Claude Code subagent calls are bounded without penalizing unrelated parallel callers. |
| [`crates/openhuman-core/src/mcp/server/stdio.rs`](./stdio.rs) | CLI entry and newline framing adapter; submits lines to TinyMCP and writes completed protocol replies. |
| [`crates/openhuman-core/src/mcp/server/stub.rs`](./stub.rs) | The `mcp`-less mirror of `run_stdio_from_cli`, `ensure_local_http`/`LocalMcpEndpoint`, and `tool_specs`: disabled-error / empty-catalog bodies so always-on callers (`core/cli.rs`, the Claude Code driver, [`tools/registry/ops.rs`](../../tools/registry/ops.rs)) keep a stable surface. |
| [`crates/openhuman-core/src/mcp/server/module_tests.rs`](./module_tests.rs), [`handler_tests.rs`](./handler_tests.rs) | Mocked bus adapter tests for positive callbacks, unavailable modules, cancellation, and preserved host error categories. TinyMCP owns protocol and transport wire fixtures. |
| [`crates/openhuman-core/src/mcp/server/tools_tests.rs`](./tools_tests.rs) | Sibling `#[cfg(test)]` suite for `tools/` (via `#[path]`). |

## Public surface

Re-exported from `mod.rs`:

- `run_stdio_from_cli(args: &[String]) -> Result<()>`: CLI entry point; builds its own tokio runtime and selects stdio vs HTTP transport.
- `run_http(config: HttpServerConfig) -> Result<()>`, `run_http_reporting` (the same, plus a oneshot that reports the bound address; `local.rs` uses it), and OpenHuman's `HttpServerConfig { bind_addr, auth_token }`: HTTP/SSE adapter (`mcp` + `http-server`).
- `ensure_local_http() -> Result<LocalMcpEndpoint>` and `LocalMcpEndpoint { addr, token }`: the loopback in-process server for the Claude Code driver.
- `current_subagent_depth()` / `HEADER_SUBAGENT_DEPTH`: the delegation-depth hop for `agent.run_subagent`.
- `tool_specs() -> Vec<McpToolSpec>` and `McpToolSpec { name, title, description, rpc_method, input_schema, annotations }`: the advertised tool catalog.

## RPC / controllers

This module exposes **no** registered core RPC methods (no `schemas.rs`, no controllers, no `openhuman.mcp_server_*` namespace). It is the **client/server-of-MCP**, not an RPC domain. Instead it *consumes* existing registered RPC methods, mapping each MCP tool to one via `all::try_invoke_registered_rpc` after validating against `all::schema_for_rpc_method`:

| MCP tool | Mapped core RPC method |
| --- | --- |
| `memory.recall` | `openhuman.memory_recall` |
| `memory.fetch` | `openhuman.memory_fetch` |
| `memory.list` | `openhuman.memory_items_list` |
| `memory.learn` | `openhuman.memory_learn` |
| `memory.forget` | `openhuman.memory_forget` |
| `web_search` | `openhuman.tools_web_search` |
| `web_answer` | `openhuman.tools_web_answer` |
| `searxng_search` | `openhuman.tools_searxng_search` |
| `core.list_tools` / `core.tool_instructions` / `agent.list_subagents` / `agent.run_subagent` | (no RPC mapping, handled in-process via `Agent` / `AgentDefinitionRegistry`) |

## Agent tools

It does **not** own any agent tools in the `tools.rs`/[`crates/openhuman-core/src/tools`](../../tools/) sense. The "tools" here are **MCP-protocol tools** advertised to external clients:

- Read-only (`ToolOperation::Read`): `core.list_tools`, `core.tool_instructions`, `agent.list_subagents`, `memory.recall`, `memory.fetch`, `memory.list`, `web_search`, `web_answer`, `searxng_search` (listed only when a search provider can serve them; `crate::search::providers`).
- Act-policy (`ToolOperation::Act`): `agent.run_subagent` (annotated destructive/open-world), and `memory.learn` (annotated non-destructive, non-idempotent, local-only) and `memory.forget` (annotated destructive/idempotent, local-only).

Argument bounds enforced in-layer: memory `limit` capped at `MEMORY_MAX_LIMIT` (100; rejected, not clamped); `memory.forget` capped at 100 ids; `filter` is validated field by field (MetaFilter of [`docs/specs/memory-v2.md`](../../../../../docs/specs/memory-v2.md)); `memory.fetch` `mode` is one of keyword/vector/hybrid but must be supported by the active engine (both launch engines: `hybrid` only); `web_search` and `web_answer` `max_results` capped at `SEARCH_MAX_RESULTS` (20).

## Events

None. This module publishes/subscribes no `DomainEvent`s and has no `bus.rs`.

## Persistence

No `store.rs`. The only durable side effect is the **MCP write-audit log**, written via `crate::mcp::audit::record_write` (the sibling `mcp::audit` domain) for every write-tool attempt, including pre-dispatch rejections. Audit rows store a PII-redacted `args_summary` (titles truncated to 128 chars; `content`/`note_text` bodies omitted, only lengths/counts kept), `client_info`, success flag, and resulting `document_id`. Audit inserts run off the hot path via `spawn_blocking` (or a thread fallback). HTTP session records (`Mcp-Session-Id` → negotiated protocol version) are kept in an in-memory map only.

## Dependencies

- `crate::core::all`: `try_invoke_registered_rpc`, `schema_for_rpc_method`, `validate_params`: dispatch MCP tool calls into the registered core RPC layer and validate params against controller schemas.
- `crate::core::logging` (`CliLogDefault`, `init_for_cli_run`): installs the stderr tracing subscriber for the MCP subprocess.
- `crate::config` (`Config`, `rpc::load_config_with_timeout`, `McpAuthConfig`/`McpClientIdentityConfig` in tests): loads config for policy/searxng gating and per-call config.
- `crate::security` (`SecurityPolicy`, `ToolOperation`): enforces read/act autonomy policy per tool call.
- `crate::agent` (`Agent`, `registry::agents::BUILTINS`, `harness::AgentDefinitionRegistry`): builds the orchestrator agent for `core.list_tools`/`core.tool_instructions`, lists/runs subagents, and cross-checks the resource catalog.
- `tinyinference_llm::tool::ToolSchema`: directly materialize model-visible tool declarations for `core.tool_instructions`.
- `tinyagents_harness::tool::prompt_tool_instructions`: render the markdown tool-use instructions block for `core.tool_instructions`.
- `crate::search::providers`: which search tools the config can serve (`tool_specs_for_config`).
- `crate::mcp::audit` (`record_write`, `NewMcpWriteRecord`, list/query helpers in tests): durable write-audit log.
- `crate::mcp::http_client::McpHttpClient`: round-trip test harness for the HTTP transport (test-only).
- `tinymcp_bus` (typed server declarations, callback and replay vocabulary, HTTP metadata); TinyMCP owns protocol validation and transport/session behavior.
- External crates: `serde_json`, `uuid`, `sha2`/`hex` (slug fallback hash), `chrono` (audit timestamps), `futures-util` (`BoxFuture`).

## Used by

- `crates/openhuman-core/src/core/cli.rs`: dispatches `mcp` / `mcp-server` subcommands to `run_stdio_from_cli`; the only production entry point for the stdio/HTTP transports. [`crates/openhuman-core/src/platform/about_app/catalog_conversation_intelligence.rs`](../../platform/about_app/catalog_conversation_intelligence.rs) lists it as `intelligence.mcp_server`.
- `crates/openhuman-core/src/inference/provider/claude_code/driver.rs`: calls `ensure_local_http` on each Claude Code turn to hand the sandboxed `claude` subprocess a loopback MCP endpoint.
- `crates/openhuman-core/src/tools/registry/ops.rs`: reads `McpToolSpec`/`tool_specs()` (via `crate::mcp::server::McpToolSpec`) to fold this server's advertised tools into the agent tool registry catalog.
- The other `mcp/` members (`mcp::host`, `mcp::registry`, `mcp::audit`) are siblings in the broader MCP feature set, not consumers of this server's code paths. `mcp::http_client` is a re-export module of `tinymcp`, used here only by the test-only `McpHttpClient` HTTP round-trip.

## Notes / gotchas

- **Not a controller-registry domain.** Don't look for `schemas.rs`/`all_controller_schemas`: exposure is via the CLI only, and tool calls re-enter the registry through `core::all`.
- **`ToolCallError` variant → JSON-RPC code is deliberate:** `InvalidParams` → `-32602` (client-actionable, including policy denials so the reason text surfaces), `Internal` → `-32603` (config load / platform failures), `ResourceNotFound` → `-32002`. Policy denials are intentionally `InvalidParams`, not `Internal`.
- **Explicit rejection over silent clamping** throughout: over-cap `k`, oversize/over-count tags, blank required strings, and unexpected arguments all error rather than being trimmed/dropped.
- **Write audit is fire-and-forget but mandatory:** rejections are audited even before config is loaded (`audit_write_rejection_without_config`), and `dispatch_write_tool` returns `Ok(tool_error(...))` (not `Err`) on RPC-handler failure so the client gets an MCP `isError` result while the failure is still recorded.
- **Protocol negotiation:** owned by TinyMCP and covered by its module protocol fixtures. The host routes HTTP methods and passes raw headers/frames without parsing MCP methods.
- **HTTP security:** bearer auth is optional (`--auth-token`); the default bind is `127.0.0.1:9300`.
- **Resource catalog parity is CI-enforced:** static prompt files are `include_str!`-embedded and the `catalog_mirrors_builtins` test fails if a built-in subagent lacks a matching `openhuman://prompts/agents/<id>` entry. Flow resource reads fetch their current text through TinyFlows Copilot.
- **`agent.run_subagent` limits:** runs a fresh single-turn agent session tagged with an `mcp:<agent_id>:<uuid>` event context.
- **stdio logging defaults to `warn`** on stderr (so failures surface in client UIs); `--verbose` → `debug`; a user-set `RUST_LOG` always wins. Stdout is reserved for protocol messages only.

## Further reading

- [Parent module README](../README.md)
- [MCP server](../../../../../docs/gitbooks/en/developing/mcp-server.md)
- [MCP servers and skills](../../../../../docs/gitbooks/en/features/integrations/mcp-and-skills.md)
