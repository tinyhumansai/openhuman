# tools/impl

Built-in implementations of the cross-cutting tool families that `tools/ops.rs`
assembles into the agent registry. Ownership rule from `AGENTS.md`: only
cross-cutting capabilities (filesystem, browser, generic process/system, generic
network, document/presentation generation, tool-surface meta helpers) live
here. Domain-owned tools (memory, cron, wallet, composio, integrations, voice,
agent sub-dispatch, ...) live in their own domain's `tools.rs` and are only
re-exported through `tools/mod.rs`. Do not add a family under `impl/` for a
domain capability.

`tools/mod.rs` mounts this directory as `#[path = "impl/mod.rs"] pub(crate) mod
implementations` (`impl` is a keyword) and glob re-exports it, so tool structs
are imported as `crate::tools::*`. `impl/mod.rs` gates `document/` and
`presentation/` behind the `documents` Cargo feature; the other five families
compile unconditionally. `meta/` is declared but not glob re-exported, so its
items are reached as `crate::tools::implementations::meta::...`.

See [`../README.md`](../README.md) for the registry, policy, and RPC layer this
feeds, and [`../../security/README.md`](../../security/README.md) for the
`SecurityPolicy` threaded through nearly every tool here (path validation,
trusted roots, egress gating).

## Families

The gates below are what `tools/ops.rs::all_tools_with_runtime` checks before
registering a tool. "Exported, not registered" means the struct is public and
tested but no production assembly site constructs it today.

| Family | Tools | Registration gate |
| --- | --- | --- |
| `system/` | `ShellTool`, `NodeExecTool`, `NpmExecTool`, `PythonExecTool`, `InstallToolTool`, `DetectToolsTool`, `CurrentTimeTool`, `ResolveTimeTool`, `ScheduleTool`, `ProxyConfigTool`, `PushoverTool`, `LspTool`, `ToolStatsTool`, `UpdateCheckTool`, `UpdateApplyTool`, `InsertSqlRecordTool`, `WorkspaceStateTool`, `RetrieveToolOutputTool`; `command_output.rs` is a shared helper, not a tool | `node_exec`/`npm_exec` and `shell`'s PATH injection need the `runtime-node` Cargo feature plus `node.enabled`; `python_exec` needs `runtime_python.enabled`; `LspTool` needs `OPENHUMAN_LSP_ENABLED` (`lsp_capability_enabled`); `ToolStatsTool` needs `learning.enabled` and `learning.tool_tracking_enabled`; `InsertSqlRecordTool` is exported, not registered; the rest are always registered |
| `filesystem/` | `FileReadTool`, `FileWriteTool`, `EditFileTool`, `ApplyPatchTool`, `GrepTool`, `GlobTool`, `ListFilesTool`, `ReadDiffTool`, `CsvExportTool`, `GitOperationsTool`, `RunLinterTool`, `RunTestsTool`, `UpdateMemoryMdTool` | always registered, except `ReadDiffTool`, `RunLinterTool`, and `RunTestsTool`, which are exported, not registered |
| `browser/` | `BrowserTool` (DOM-snapshot automation with pluggable backend: `agent_browser`, `playwright`, `rust_native`, `computer_use`, `auto`), `BrowserOpenTool`, `ImageInfoTool` | `BrowserTool`/`BrowserOpenTool` need `browser.enabled`; `ImageInfoTool` is always registered. The browser host allowlist is `browser_allowed_domains(http_request.allowed_domains)`, which strips the `"*"` wildcard; allow-all needs `OPENHUMAN_BROWSER_ALLOW_ALL`. Playwright ships as `playwright_backend.rs` + `include_str!("playwright_runner.mjs")`; `native_backend.rs` (fantoccini) compiles only with the `browser-native` Cargo feature |
| `network/` | `HttpRequestTool`, `WebFetchTool`, `CurlTool`, `GitbooksSearchTool`, `GitbooksGetPageTool`, `GmailUnsubscribeTool`, and under the `mcp` Cargo feature `McpListServersTool`, `McpListToolsTool`, `McpCallTool`, `McpSetupSearchTool`, `McpSetupGetTool`, `McpSetupInstallAndConnectTool`, `McpSetupRequestSecretTool`, `McpSetupTestConnectionTool`; `url_guard.rs` is the shared SSRF/allowlist validator, not a tool | `http_request`, `web_fetch`, `curl`, `gmail_unsubscribe` always registered; `gitbooks_*` need `gitbooks.enabled` and, when the active profile sets an `mcp_allowlist`, an entry named `gitbooks`; the `mcp_setup_*` tools register whenever the `mcp` feature is on; `mcp_list_servers`/`mcp_list_tools`/`mcp_call_tool` register only when the static `[[mcp_client.servers]]` registry (after the profile allowlist) is non-empty |
| `document/` (feature `documents`) | `DocumentTool` (`generate_document`) | always registered when compiled. Builds `.docx` bytes via the pure-Rust `engine` module (`docx-rs`, no subprocess) inside `spawn_blocking` + `tokio::time::timeout`; allocates the artifact with `agent::artifacts::create_artifact` (kind `Document`), persists `args.json` next to it for the failed-card Retry path, then `finalize_artifact` or `fail_artifact` |
| `presentation/` (feature `documents`) | `PresentationTool` (`generate_presentation`) | always registered when compiled. Same shape as `document/` with `ppt-rs` producing `.pptx` and kind `Presentation` |
| `meta/` | `ToolSearchTool` (`tool_search`, name constant `TOOL_SEARCH_NAME`), `ToolSearchIndex`, `ToolSearchHandle`, `strip_deferred_from_visible`, `bind_tool_search_index`; `collapse` helpers (`resolve`, `merge_action_schemas`, `args_without_action`, `strictest_permission`, `any_external_effect`, `unknown_action_message`, `CollapsedAction`) | not registered: nothing in `tools/ops.rs` or the session builder constructs `ToolSearchTool`, and `strip_deferred_from_visible`/`bind_tool_search_index` have no production caller, so `ToolExposure::Deferred` (declared by e.g. `integrations/tools/stock_prices.rs`) is not enforced today; only `Hidden` is filtered in `agent/session_host/builder/factory.rs`. The `collapse` helpers are used by `cron/tools/collapsed.rs` and `memory/tools/collapsed.rs` to merge a multi-action tool's per-action schemas and permissions into one entry |

## Notes

- `document/` and `presentation/` are kept structurally parallel (validate
  input, allocate artifact, generate bytes off the async runtime, finalize or
  fail the artifact) so further artifact-producing tools follow the same shape.
- `meta/` sits outside `system/` because it is the model introspecting its own
  tool surface, not a host capability.
- `system/mod.rs` and `filesystem/mod.rs` each carry a
  `security_for_tool_context` that grants the run's TinyAgents workspace
  descriptor as `action_dir` plus a `ReadWrite` trusted root on a per-call
  clone of `SecurityPolicy`. They must stay in step; `is_always_forbidden` and
  `is_workspace_internal_path` are evaluated before any trusted-root shortcut.
