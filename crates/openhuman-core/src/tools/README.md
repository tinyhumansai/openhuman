# tools

The agent tool layer. Defines the core [`Tool`] trait every agent-callable capability implements, assembles the **default tool registry** consumed by the agent harness, hosts the cross-cutting built-in tool implementations (filesystem, browser, generic system/network/document/presentation), and exposes a small allowlist of tool operations over JSON-RPC for the Tauri shell. Domain-owned tools (cron, memory, wallet, composio, skills, etc.) live in their own domains and are re-exported here so a single `crate::tools::*` import surfaces the full set. The crate's lib target is `openhuman_core` (`[lib] name = "openhuman_core"`), so external callers (e.g. `openhuman-embed`) reach this module as `openhuman_core::tools::*`; everything below uses in-crate `crate::` paths.

## Responsibilities

- Use the [`tinytools::Tool`] async trait and its supporting value types (`ToolResult`, `ToolSpec`, `PermissionLevel`, `ToolScope`, `ToolCategory`, `ToolCallOptions`, `ToolExposure`) directly from `tinytools`.
- Assemble the registry the agent loop runs against — `default_tools[_with_runtime]` (minimal: shell + file read/write) and `all_tools[_with_runtime]` (full, config-gated set).
- Gate registration on config flags / env (`browser.enabled`, `node.enabled`, `runtime_python.enabled`, `learning.*`, `integrations.*`, `search.engine`, `gitbooks.enabled`, MCP registry presence, `OPENHUMAN_LSP_ENABLED`).
- Own the cross-cutting built-in tool impls under `impl/` (filesystem, browser, generic system, generic network, meta, and the `documents`-gated document/presentation tools).
- Provide the pre-execution [`ToolPolicy`] middleware (allow/deny gate) and the default allow-all policy.
- Normalize tool JSON schemas for provider compatibility (`SchemaCleanr`).
- Synthesize per-subagent orchestrator tools at agent-build time (`orchestrator_tools`).
- Wrap runtime-generated capability tools (`generated`).
- Filter the registry by user tool-toggle preferences (`user_filter`).
- Own tool-call lifecycle state and failure classification (`status`), on-demand schema disclosure (`toolpacks`), the process-wide tool execution timeout (`timeout`), the read-only cross-surface discovery registry (`registry`), and per-session tool-boundary policy (`agent_policy`) — each documented in its own sibling README.
- Expose a JSON-RPC `tools` controller allowlist (`openhuman.tools_*` on the wire) for Tauri-driven flows (onboarding-style orchestration in the renderer).
- Browser-allowlist derivation: narrow the browser host list from the unified fetch allowlist (`browser_allowed_domains`, strips `"*"`).

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/tools/mod.rs` | Export hub. Declares submodules, re-exports the built-in impls and every domain-owned tool set, and the `all_tools_*` controller pair. |
| `crates/openhuman-core/src/tools/host_extensions.rs` | OpenHuman-only readers over erased `host_extension` slots: `pack_registry_handle`, `delegation_target`, and `generated_runtime_context`. Import the shared `Tool` vocabulary from `tinytools` directly. |
| `crates/openhuman-core/src/tools/ops.rs` | Registry assembly: `default_tools`, `default_tools_with_runtime`, `all_tools`, `all_tools_with_runtime`, `browser_allowed_domains`. All config-gating logic lives here. |
| `crates/openhuman-core/src/tools/schemas.rs` (thin shell over the `schemas/` submodule: `apify.rs`, `composio.rs`, `registry.rs`, `web_search.rs`) | JSON-RPC `tools` namespace controllers + `handle_*` fns. `all_controller_schemas` / `all_registered_controllers` (re-exported as `all_tools_*`). |
| `crates/openhuman-core/src/tools/policy.rs` | `ToolPolicy` trait + `PolicyDecision` (`Allow`/`Deny`) + allow-all `DefaultToolPolicy`. Evaluated on the agent hot path before each `execute()`. |
| `crates/openhuman-core/src/tools/schema.rs` | Re-exports `SchemaCleanr`, `CleaningStrategy` and `GEMINI_UNSUPPORTED_KEYWORDS` from `tinyagents_harness::tool` (local `$ref` resolution, provider-rejected keyword stripping, literal-union flattening). The only in-crate caller is `generated.rs`, which runs `SchemaCleanr::validate` on generated tool schemas at admission. |
| `crates/openhuman-core/src/tools/orchestrator_tools.rs` | Synthesizes named per-subagent tools from the orchestrator's `subagents = [...]` definition; collapses skill wildcards into `delegate_to_integrations_agent`. |
| `crates/openhuman-core/src/tools/generated.rs` | `GeneratedToolDefinition` + wrapper for runtime/profile-supplied generated capability tools (provider/capability/risk metadata for policy). |
| `crates/openhuman-core/src/tools/user_filter.rs` | `filter_tools_by_user_preference` + UI-toggle-ID → Rust-tool-name map. Unmapped tools are always retained. |
| [`crates/openhuman-core/src/tools/status/`](status/mod.rs) | Tool-call lifecycle state (`ToolLifecycleState`) and human-readable failure classification (`ToolFailureClass`, `classify`). Pure data/logic; no persistence, no RPC. |
| [`crates/openhuman-core/src/tools/toolpacks/`](toolpacks/README.md) | On-demand tool disclosure: keeps a pack's tools constructed but unadvertised until `UseSkillTool` (`use_skill`) renders or invokes one, trimming per-turn schema token cost. Also home of `ToolGroups`/`GroupMode`, which `openhuman-embed` re-exports. |
| [`crates/openhuman-core/src/tools/timeout/`](timeout/README.md) | Process-wide tool execution timeout (`OPENHUMAN_TOOL_TIMEOUT_SECS` → config → `120`s default); scripting tools (`shell`/`node_exec`/`npm_exec`/`python_exec`) return `ToolTimeout::Unbounded` unless given an explicit `timeout_secs`. |
| [`crates/openhuman-core/src/tools/registry/`](registry/README.md) | Unified read-only discovery registry across MCP stdio, JSON-RPC controller, and connected MCP client tools, plus policy/tool-visibility diagnostics. |
| [`crates/openhuman-core/src/tools/agent_policy/`](agent_policy/README.md) | Per-session tool boundary: classifies every tool against a channel's permission ceiling into allow/require-approval/deny/hide, and renders the prompt-visible boundary section. |
| [`crates/openhuman-core/src/tools/impl/mod.rs`](impl/README.md) | Aggregates the built-in tool families; glob re-exports `browser`, `filesystem`, `network`, `system` and the two `documents`-gated tools. `meta` is declared `pub` but not glob re-exported — reach it as `implementations::meta`. |
| `crates/openhuman-core/src/tools/impl/filesystem/` | Tools `file_read`, `file_write`, `edit`, `apply_patch`, `grep`, `glob`, `list`, `read_diff`, `csv_export`, `git_operations`, `run_linter`, `run_tests`, `update_memory_md`. Helper modules (not tools): `git_operations_config`/`git_operations_render`, and `write_sink` (the injectable filesystem-write seam tests use to provoke OS refusals). |
| `crates/openhuman-core/src/tools/impl/browser/` | `browser` (DOM-snapshot automation, pluggable backend), `browser_open`, `image_info`, Playwright backend. |
| `crates/openhuman-core/src/tools/impl/system/` | Tools `shell`, `node_exec`, `npm_exec`, `python_exec`, `install_tool`, `detect_tools`, `current_time`, `resolve_time`, `schedule`, `proxy_config`, `pushover`, `lsp`, `tool_stats`, `update_check`, `update_apply`, `insert_sql_record`, `read_workspace_state`, `retrieve_tool_output`. Helper module (not a tool): `command_output`, the shared exit-code/stdout/stderr formatter for the shell family. |
| `crates/openhuman-core/src/tools/impl/network/` | Tools `http_request`, `web_fetch`, `curl`, `gitbooks_search`/`gitbooks_get_page`, `mcp_list_servers`/`mcp_list_tools`/`mcp_call_tool` and the five `mcp_setup_*` tools (both `mcp`-feature gated), `gmail_unsubscribe`. Helper module: `url_guard` (host allowlist matching, private-address rejection, `validate_url`). |
| `crates/openhuman-core/src/tools/impl/meta/` | Tools *about* the tool surface itself: `tool_search` (the lookup half of `ToolExposure::Deferred`) and `collapse` (multi-action schema/permission merging helpers). |
| `crates/openhuman-core/src/tools/impl/document/` (`documents` feature) | `DocumentTool` (`generate_document`) — structured document generation/editing engine. |
| `crates/openhuman-core/src/tools/impl/presentation/` (`documents` feature) | `PresentationTool` (`generate_presentation`) — structured slide-deck generation engine. |
| `crates/openhuman-core/src/search/` | Search engine registry and search-owned agent tools such as `web_search`. |
| `*_tests.rs` / `#[cfg(test)] mod tests` | Co-located/sibling unit tests across the module. |

## Public surface

- Trait + types: `Tool`, `ToolSpec`, `ToolResult`, `ToolContent`, `ToolExposure`, `PermissionLevel`, `ToolScope`, `ToolCategory`, `ToolCallOptions`.
- Registry constructors: `ops::default_tools`, `ops::default_tools_with_runtime`, `ops::all_tools`, `ops::all_tools_with_runtime`.
- Policy: `ToolPolicy`, `DefaultToolPolicy`, `PolicyDecision`.
- Schema: `SchemaCleanr`, `CleaningStrategy`.
- Controllers: `all_tools_controller_schemas`, `all_tools_registered_controllers`.
- All built-in tool structs (e.g. `ShellTool`, `FileReadTool`, `EditFileTool`, `GrepTool`, `BrowserTool`, `HttpRequestTool`, `CurlTool`, `DocumentTool`, `PresentationTool`, …) via `pub use implementations::*`, plus every re-exported domain tool set listed in `mod.rs` (agent, config, cron, desktop dashboard, flows, integrations, mcp registry, memory, platform, search, security, skills, threads todos, voice audio toolkit, web3 wallet).
- `filter_tools_by_user_preference` (crate-internal).

## RPC / controllers

Namespace `tools` (wired into `crates/openhuman-core/src/core/all.rs` via `all_tools_registered_controllers` / `all_tools_controller_schemas`). A deliberately small allowlist for Tauri-driven flows; everything else stays agent-only. Wire method names follow the registry's `openhuman.<namespace>_<function>` rule (`core::all::rpc_method_name`); there are no `tools_*` entries in `core/legacy_aliases.rs`. (`tools.web_search`-style dotted ids are `tool_registry` `tool_id`s, not RPC methods.)

| Method | Purpose |
| --- | --- |
| `openhuman.tools_composio_execute` | Run a Composio action via the mode-aware client factory (backend-proxied or direct). |
| `openhuman.tools_web_search` | Web search via the backend `/agent-integrations/parallel/search` proxy; structured results plus the resolved provider. |
| `openhuman.tools_seltz_search` | Seltz web search (gated on `seltz.enabled`). |
| `openhuman.tools_querit_search` | Querit web search (gated on `search.querit` having a key). |
| `openhuman.tools_searxng_search` | Self-hosted SearXNG search (gated on `searxng.enabled`). |
| `openhuman.tools_apify_linkedin_scrape` | Apify LinkedIn profile scrape → raw JSON + rendered markdown. |

Handlers load config via `config::rpc::load_config_with_timeout`, build the backend integration client where needed, and return `RpcOutcome`.

## Agent tools

This module **owns** the cross-cutting built-in tools (the only ones that belong here per the repo's tool-ownership rule):

- **Filesystem**: `file_read`, `file_write`, `edit`, `apply_patch`, `grep`, `glob`, `list`, `read_diff`, `csv_export`, `git_operations`, `run_linter`, `run_tests`, `update_memory_md`.
- **System/process**: `shell`, `node_exec`, `npm_exec`, `python_exec`, `install_tool`, `detect_tools`, `current_time`, `resolve_time`, `schedule`, `proxy_config`, `pushover`, `lsp`, `tool_stats`, `update_check`, `update_apply`, `insert_sql_record`, `read_workspace_state`, `retrieve_tool_output`.
- **Browser**: `browser`, `browser_open`, `image_info`.
- **Generic network**: `http_request`, `web_fetch`, `curl`, `gitbooks_search`/`gitbooks_get_page`, MCP bridge (`mcp_list_servers`/`mcp_list_tools`/`mcp_call_tool`), `mcp_setup_*` tools, `gmail_unsubscribe`.
- **Meta**: `tool_search` (deferred-tool lookup) and the `collapse` multi-action helpers used by other tools' schema merging.
- **Documents** (`documents` feature): `generate_document` (`DocumentTool`), `generate_presentation` (`PresentationTool`).
- **Search**: `web_search` and provider-specific search families are registered by `crate::search`; `search.engine = "disabled"` suppresses this surface entirely.

Domain-owned tools (memory, cron, wallet, composio, integrations, skills, voice::audio_toolkit, agent sub-dispatch like `spawn_subagent`/`spawn_async_subagent`/`delegate`/`todo`/`plan_exit`/`run_skill`) are **registered** in `all_tools` but implemented in their respective domains and only re-exported through this module.

## Events

None. This module has no `bus.rs` and registers no `EventHandler`. Approval coordination is via the `Tool::external_effect[_with_args]` hooks that the agent harness reads to route calls through the `ApprovalGate`; the gate itself lives in `crate::security::approval`.

## Persistence

None. No `store.rs`; the module holds no persisted state. Tools that persist (memory, cron, etc.) do so through their own domains.

## Dependencies

- `crate::agent` — `host_runtime` (`RuntimeAdapter`/`NativeRuntime`), `tool_policy::GeneratedToolRuntimeContext`, harness definitions (`AgentDefinition`, `SubagentEntry`) for orchestrator tool synthesis, and the agent-owned dispatch tools re-exported here.
- `crate::config` — `Config`, `BrowserConfig`, `HttpRequestConfig`, `DelegateAgentConfig`; drives all registration gating and `config::rpc::load_config_with_timeout` in RPC handlers.
- `crate::search` — active search engine registry and search-owned tool implementations.
- `crate::security` — `SecurityPolicy` (host/path/command gating threaded into nearly every tool) + `AuditLogger`.
- `crate::memory` — `memory::ops::guard::active_memory_guard` (read by `tool_stats`) and the memory-owned tool sets re-exported here; the registry takes no `Memory` handle itself.
- `crate::integrations` — `build_client` backend HTTP client + the integration tool structs (apify, brave, parallel, stock, twilio, tinyfish, google_places, querit, seltz, searxng).
- `crate::integrations::composio` — `all_composio_agent_tools`, mode-aware client (`create_composio_client`) for `openhuman.tools_composio_execute`.
- `crate::runtime::javascript` / `crate::runtime::python` — `NodeBootstrap` shared by shell/node_exec/npm_exec (behind `runtime-node`), `PythonBootstrap` for `python_exec`.
- `crate::mcp::config_servers` (`McpServerRegistry` behind the `mcp_*` bridge tools and the gitbooks MCP source) and `crate::mcp::registry` (tool re-exports; `connections` for the discovery registry) — `mcp` feature.
- `tinytools` (vendored via `vendor/tinyagents/`) — the `Tool` trait itself and its vocabulary, imported directly by each consumer.
- `crate::skills` — skill-run spawning and skill-owned tools (`skills` feature).
- `crate::agent::learning` — LinkedIn enrichment scrape/render for the Apify RPC handler.
- `crate::web3::wallet`, `crate::cron`, `crate::voice::audio_toolkit` — domain-owned tools re-exported and registered (behind their respective features).
- `crate::inference::provider` — `ProviderRuntimeOptions` handed to `DelegateTool`.
- `crate::security::approval`, `crate::agent::context`, `crate::security::credentials`, `crate::platform::update`, `crate::util` — supporting types used by individual tools.
- `core::all` — `ControllerSchema`, `FieldSchema`, `TypeSchema`, `RegisteredController`, `ControllerFuture` for the RPC controller surface.

## Used by

- `crates/openhuman-core/src/core/all.rs` — registers the `tools` RPC controllers + schemas.
- `crate::agent` harness (`agent/session_host/builder/`, `subagent_runner`, `agent/tools/*`) and the `crate::agent::tinyagents` seam (`CanonicalSharedToolAdapter`, `ToolPolicyMiddleware`) — primary consumers; build the registry and execute/police tools on the tinyagents harness path.
- `crate::channels::runtime::dispatch::routing` — calls `orchestrator_tools::collect_orchestrator_tools` to build per-subagent orchestrator tool sets.
- `crate::tools::agent_policy`, `crate::security::approval` — read tool metadata (category, external-effect) for policy/approval decisions.
- `crate::tools::registry` (reads `all_tools_controller_schemas`) and `crate::mcp::server` (reuses `SEARXNG_MAX_RESULTS` / `normalize_categories` for its stdio tool specs).
- `openhuman-embed` — re-exports `toolpacks::{ToolGroups, GroupMode}` as its tool-visibility control.

## Notes / gotchas

- **Ownership rule**: only genuinely cross-cutting tool families (filesystem, browser, generic system/network, meta, and the `documents`-gated document/presentation tools) belong in `impl/`. New domain tools go in the owning domain's `tools.rs` and are re-exported via `mod.rs` — do not add them under `impl/`.
- **One unified `ToolResult`**: every tool imports it from `tinytools`, so every tool uses the same type.
- **Browser allowlist is fail-safe**: the browser shares `http_request.allowed_domains` but `browser_allowed_domains` strips the `"*"` wildcard — unifying can only narrow browser reach. Allow-all stays behind `OPENHUMAN_BROWSER_ALLOW_ALL`.
- **Node tools are co-gated**: `shell`, `node_exec`, and `npm_exec` share one memoised `NodeBootstrap`; with `node.enabled = false` (or the `runtime-node` feature off), node/npm tools are not registered and shell skips PATH injection.
- **`external_effect_with_args`** is the hook the harness checks at the gate-decision point (not the arg-less variant) — override it for per-call gating (e.g. composio `execute` vs `list`).
- **`PermissionLevel` ordering is load-bearing**: the runtime compares `<` to reject tools above a channel's max; `permission_level()` should return the *minimum* level across a multi-action tool, with `permission_level_with_args` doing the per-call check.
- **`is_concurrency_safe` is still advisory in practice**: it is mapped to `ToolRuntime.idempotent` in `agent/tinyagents/tools.rs`, and tinyagents only runs a multi-call batch concurrently when no tool-wrap middleware is registered. OpenHuman's approval and scope gates are `wrap_tool` middlewares, so its turns take the serial path.
- **RPC surface is intentionally tiny** (6 methods) — anything not in `schemas.rs`/`schemas/` is agent-only.
- **Search engine is a single selector**: `search.engine` (`disabled`/`managed`/`parallel`/`brave`/`querit`/`exa`/`tavily`; a BYO engine without a key falls back to `managed`) chooses which `web_search`-family tools register; legacy `seltz`/`searxng` blocks are parsed but no longer auto-register agent tools (they remain reachable via their RPC handlers).
