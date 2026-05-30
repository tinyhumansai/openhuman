# tools

The agent tool layer. Defines the core [`Tool`] trait every agent-callable capability implements, assembles the **default tool registry** consumed by the agent harness, hosts the cross-cutting built-in tool implementations (filesystem, browser/computer, generic system/network), and exposes a small allowlist of tool operations over JSON-RPC for the Tauri shell. Domain-owned tools (cron, memory, wallet, composio, codegraph, etc.) live in their own domains and are re-exported here so a single `openhuman::tools::*` import surfaces the full set.

## Responsibilities

- Define the [`Tool`] async trait and its supporting value types (`ToolResult`, `ToolSpec`, `PermissionLevel`, `ToolScope`, `ToolCategory`, `ToolCallOptions`).
- Assemble the registry the agent loop runs against — `default_tools[_with_runtime]` (minimal: shell + file read/write) and `all_tools[_with_runtime]` (full, config-gated set).
- Gate registration on config flags / env (`browser.enabled`, `node.enabled`, `computer_control.enabled`, `learning.*`, `integrations.*`, `search.engine`, `gitbooks.enabled`, MCP registry presence, `OPENHUMAN_LSP_ENABLED`).
- Own the cross-cutting built-in tool impls under `impl/` (filesystem, browser, computer, generic system, generic network).
- Provide the pre-execution [`ToolPolicy`] middleware (allow/deny gate) and the default allow-all policy.
- Normalize tool JSON schemas for provider compatibility (`SchemaCleanr`).
- Synthesize per-subagent orchestrator tools at agent-build time (`orchestrator_tools`).
- Wrap runtime-generated capability tools (`generated`).
- Filter the registry by user tool-toggle preferences (`user_filter`).
- Expose a JSON-RPC `tools.*` controller allowlist for Tauri-driven flows (onboarding-style orchestration in the renderer).
- Browser-allowlist derivation: narrow the browser host list from the unified fetch allowlist (`browser_allowed_domains`, strips `"*"`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/tools/mod.rs` | Export hub. Declares submodules, re-exports built-in impls and the domain-owned tool sets (`agent`, `audio_toolkit`, `codegraph`, `composio`, `cron`, `integrations`, `memory`, `wallet`, `whatsapp_data`), and the `all_tools_*` controller pair. |
| `src/openhuman/tools/traits.rs` | The `Tool` trait + `ToolResult`/`ToolContent` re-export, `ToolSpec`, `PermissionLevel`, `ToolScope`, `ToolCategory`, `ToolCallOptions`. Defines per-tool hooks: permission level (incl. args-aware), scope, category, concurrency safety, `external_effect[_with_args]` (drives approval gating), `max_result_size_chars`, markdown preference, generated-runtime context. |
| `src/openhuman/tools/ops.rs` | Registry assembly: `default_tools`, `default_tools_with_runtime`, `all_tools`, `all_tools_with_runtime`, `browser_allowed_domains`. All config-gating logic lives here. |
| `src/openhuman/tools/schemas.rs` | JSON-RPC `tools` namespace controllers + `handle_*` fns. `all_controller_schemas` / `all_registered_controllers` (re-exported as `all_tools_*`). |
| `src/openhuman/tools/policy.rs` | `ToolPolicy` trait + `PolicyDecision` (`Allow`/`Deny`) + allow-all `DefaultToolPolicy`. Evaluated on the agent hot path before each `execute()`. |
| `src/openhuman/tools/schema.rs` | `SchemaCleanr` / `CleaningStrategy` — per-provider JSON-Schema normalization (Gemini keyword stripping, `$ref` resolution, union flattening, cycle detection). |
| `src/openhuman/tools/orchestrator_tools.rs` | Synthesizes named per-subagent tools from the orchestrator's `subagents = [...]` definition; collapses skill wildcards into `delegate_to_integrations_agent`. |
| `src/openhuman/tools/generated.rs` | `GeneratedToolDefinition` + wrapper for runtime/profile-supplied generated capability tools (provider/capability/risk metadata for policy). |
| `src/openhuman/tools/user_filter.rs` | `filter_tools_by_user_preference` + UI-toggle-ID → Rust-tool-name map. Unmapped tools are always retained. |
| `src/openhuman/tools/local_cli.rs` | Local CLI helpers (e.g. screenshot wrappers) that run tools against workspace config without the RPC server. |
| `src/openhuman/tools/impl/mod.rs` | Aggregates and re-exports the built-in tool families. |
| `src/openhuman/tools/impl/filesystem/` | `file_read`, `file_write`, `edit_file`, `apply_patch`, `grep`, `glob_search`, `list_files`, `read_diff`, `csv_export`, `git_operations`, `run_linter`, `run_tests`, `update_memory_md`. |
| `src/openhuman/tools/impl/browser/` | `browser` (full automation, pluggable backend), `browser_open`, `screenshot`, `image_info`, image output, action parser, native backend, security. |
| `src/openhuman/tools/impl/computer/` | `mouse`, `keyboard` (native control, default-off), human-path resolution. |
| `src/openhuman/tools/impl/network/` | `http_request`, `web_fetch`, `curl`, `gitbooks` (search/get-page), `mcp` (list servers/tools, call), `mcp_setup` (5 setup-agent tools), `polymarket` (+ orders, CLOB auth), `gmail_unsubscribe`, `url_guard`. |
| `src/openhuman/search/` | Search engine registry and search-owned agent tools such as `web_search`. |
| `src/openhuman/tools/impl/system/` | `shell`, `node_exec`, `npm_exec`, `install_tool`, `detect_tools`, `current_time`, `schedule`, `proxy_config`, `pushover`, `lsp`, `tool_stats`, `update_check`, `update_apply`, `insert_sql_record`, `workspace_state`. |
| `*_tests.rs` / `#[cfg(test)] mod tests` | Co-located/sibling unit tests across the module. |

## Public surface

- Trait + types: `Tool`, `ToolSpec`, `ToolResult`, `ToolContent`, `PermissionLevel`, `ToolScope`, `ToolCategory`, `ToolCallOptions`.
- Registry constructors: `default_tools`, `default_tools_with_runtime`, `all_tools`, `all_tools_with_runtime` (via `pub use ops::*`).
- Policy: `ToolPolicy`, `DefaultToolPolicy`, `PolicyDecision`.
- Schema: `SchemaCleanr`, `CleaningStrategy`.
- Controllers: `all_tools_controller_schemas`, `all_tools_registered_controllers`.
- All built-in tool structs (e.g. `ShellTool`, `FileReadTool`, `EditFileTool`, `GrepTool`, `BrowserTool`, `HttpRequestTool`, `CurlTool`, `WebSearchTool`, `LspTool`, …) via `pub use implementations::*`, plus re-exported domain tool sets such as `openhuman::search::tools::*`.
- `filter_tools_by_user_preference` (crate-internal).

## RPC / controllers

Namespace `tools` (wired into `src/core/all.rs` via `all_tools_registered_controllers` / `all_tools_controller_schemas`). A deliberately small allowlist for Tauri-driven flows; everything else stays agent-only.

| Method | Purpose |
| --- | --- |
| `openhuman.tools_composio_execute` | Run a Composio action via the mode-aware factory (backend-proxied or direct). |
| `openhuman.tools_web_search` | Web search via the backend Parallel proxy; structured results. |
| `openhuman.tools_seltz_search` | Seltz web search (gated on `seltz.enabled`). |
| `openhuman.tools_querit_search` | Querit web search (gated on a configured Querit key). |
| `openhuman.tools_searxng_search` | Self-hosted SearXNG search (gated on `searxng.enabled`). |
| `openhuman.tools_apify_linkedin_scrape` | Apify LinkedIn profile scrape → raw JSON + rendered markdown. |
| `openhuman.tools_polymarket_execute` | Polymarket action dispatch (Gamma + CLOB; reads and trading writes), gated on `integrations.polymarket.enabled`. |

Handlers load config via `config::rpc::load_config_with_timeout`, build the backend integration client where needed, and return `RpcOutcome`.

## Agent tools

This module **owns** the cross-cutting built-in tools (the only ones that belong here per the repo's tool-ownership rule):

- **Filesystem**: `file_read`, `file_write`, `edit_file`, `apply_patch`, `grep`, `glob`/`glob_search`, `list_files`, `read_diff`, `csv_export`, `git_operations`, `run_linter`, `run_tests`, `update_memory_md`.
- **System/process**: `shell`, `node_exec`, `npm_exec`, `install_tool`, `detect_tools`, `current_time`, `schedule`, `proxy_config`, `pushover`, `lsp`, `tool_stats`, `update_check`, `update_apply`.
- **Browser/computer**: `browser`, `browser_open`, `screenshot`, `image_info`, `mouse`, `keyboard`.
- **Generic network**: `http_request`, `web_fetch`, `curl`, `gitbooks_search`/`gitbooks_get_page`, MCP bridge (`mcp` list/call), `mcp_setup` tools, `gmail_unsubscribe`.
- **Search**: `web_search` and provider-specific search families are registered by `openhuman::search::registry`; `search.engine = "disabled"` suppresses this surface entirely.

Domain-owned tools (memory, cron, wallet, composio, codegraph, integrations, whatsapp_data, audio_toolkit, agent sub-dispatch like `spawn_subagent`/`delegate`/`todo`/`plan_exit`/`run_skill`) are **registered** in `all_tools` but implemented in their respective domains and only re-exported through this module.

## Events

None. This module has no `bus.rs` and registers no `EventHandler`. Approval coordination is via the `Tool::external_effect[_with_args]` hooks that the agent harness reads to route calls through the `ApprovalGate`; the gate itself lives in `openhuman::approval`.

## Persistence

None. No `store.rs`; the module holds no persisted state. Tools that persist (memory, cron, etc.) do so through their own domains.

## Dependencies

- `openhuman::agent` — `host_runtime` (`RuntimeAdapter`/`NativeRuntime`), `tool_policy::GeneratedToolRuntimeContext`, harness definitions (`AgentDefinition`, `SubagentEntry`) for orchestrator tool synthesis, and the agent-owned dispatch tools re-exported here.
- `openhuman::config` — `Config`, `BrowserConfig`, `HttpRequestConfig`, `SearchEngine`, `DelegateAgentConfig`; drives all registration gating and `config::rpc::load_config_with_timeout` in RPC handlers.
- `openhuman::search` — active search engine registry and search-owned tool implementations.
- `openhuman::security` — `SecurityPolicy` (host/path/command gating threaded into nearly every tool) + `AuditLogger`.
- `openhuman::memory` — `Memory` trait, injected into memory/preference/stats tools.
- `openhuman::integrations` — `build_client` backend HTTP client + the integration tool structs (apify, brave, parallel, stock, twilio, tinyfish, google_places, querit, seltz, searxng).
- `openhuman::composio` — `all_composio_agent_tools`, mode-aware client for `tools.composio_execute`.
- `openhuman::javascript` — `NodeBootstrap` shared by shell/node_exec/npm_exec.
- `openhuman::mcp_client` / `openhuman::mcp_registry` — generic remote MCP server registry + bridge tools.
- `openhuman::skills` — `skills::types::{ToolResult, ToolContent}` (the unified result type) + skill-run spawning.
- `openhuman::learning` — LinkedIn enrichment scrape/render for the Apify RPC handler.
- `openhuman::wallet`, `openhuman::cron`, `openhuman::codegraph`, `openhuman::audio_toolkit`, `openhuman::whatsapp_data` — domain-owned tools re-exported and registered.
- `openhuman::approval`, `openhuman::context`, `openhuman::credentials`, `openhuman::update`, `openhuman::util` — supporting types used by individual tools.
- `core::all` — `ControllerSchema`, `FieldSchema`, `TypeSchema`, `RegisteredController`, `ControllerFuture` for the RPC controller surface.

## Used by

- `src/core/all.rs` — registers the `tools` RPC controllers + schemas.
- `openhuman::agent` harness (`session/builder`, `tool_loop`, `dispatcher`, `subagent_runner`, `agent/tools/*`) — primary consumer; builds and runs the registry, applies the `ToolPolicy`.
- `openhuman::channels`, `openhuman::routing`, `openhuman::inference::provider` — build tool sets / clean schemas per provider.
- `openhuman::agent_tool_policy`, `openhuman::approval` — read tool metadata (category, external-effect) for policy/approval decisions.
- `openhuman::tool_registry`, `openhuman::runtime_node`, `openhuman::mcp_server` — registry/exposure consumers.
- Many domains re-export their own tools through this module (cron, memory, wallet, composio, integrations, codegraph, whatsapp_data, audio_toolkit).

## Notes / gotchas

- **Ownership rule**: only genuinely cross-cutting tool families (filesystem, browser/computer, generic system/network) belong in `impl/`. New domain tools go in the owning domain's `tools.rs` and are re-exported via `mod.rs` — do not add them under `impl/`.
- **One unified `ToolResult`**: `traits.rs` re-exports it from `skills::types` so every tool uses the same type.
- **Browser allowlist is fail-safe**: the browser shares `http_request.allowed_domains` but `browser_allowed_domains` strips the `"*"` wildcard — unifying can only narrow browser reach. Allow-all stays behind `OPENHUMAN_BROWSER_ALLOW_ALL`.
- **Node tools are co-gated**: `shell`, `node_exec`, and `npm_exec` share one memoised `NodeBootstrap`; with `node.enabled = false`, node/npm tools are not registered and shell skips PATH injection.
- **`external_effect_with_args`** is the hook the harness checks at the gate-decision point (not the arg-less variant) — override it for per-call gating (e.g. composio `execute` vs `list`).
- **`PermissionLevel` ordering is load-bearing**: the runtime compares `<` to reject tools above a channel's max; `permission_level()` should return the *minimum* level across a multi-action tool, with `permission_level_with_args` doing the per-call check.
- **`is_concurrency_safe` is advisory today**: the harness tool loop currently runs calls serially regardless; annotating tools is forward-prep for the parallel-dispatch refactor.
- **RPC surface is intentionally tiny** (7 methods) — anything not in `schemas.rs` is agent-only.
- **Search engine is a single selector**: `search.engine` (`managed`/`parallel`/`brave`/`querit`) chooses which `web_search`-family tools register; legacy `seltz`/`searxng` blocks are parsed but no longer auto-register agent tools (they remain reachable via their RPC handlers).
