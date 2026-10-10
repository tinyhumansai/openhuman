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
are imported as `crate::tools::*`. `impl/mod.rs` gates [`document/`](./document/) and
[`presentation/`](./presentation/) behind the `documents` Cargo feature; the other five families
compile unconditionally. [`meta/`](./meta/) is declared but not glob re-exported, so its
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
| [`system/`](./system/) | `ShellTool`, `InstallToolTool`, `DetectToolsTool`, `CurrentTimeTool`, `ResolveTimeTool`, `ScheduleTool`, `ProxyConfigTool`, `PushoverTool`, `LspTool`, update tools, workspace and retrieval helpers. |
| [`filesystem/`](./filesystem/) | Host adapter only: `SecurityPolicy` implements `tinytools_std::filesystem::FsGate`. The tools (`FileReadTool`, `FileWriteTool`, `EditFileTool`, `ApplyPatchTool`, `GrepTool`, `GlobTool`, `ListFilesTool`, `ReadDiffTool`, `CsvExportTool`, `GitOperationsTool`, `RunLinterTool`, `RunTestsTool`, `UpdateMemoryMdTool`) live in `tinytools_std::filesystem` and are imported directly by `tools/ops.rs` | always registered, except `ReadDiffTool`, `RunLinterTool`, and `RunTestsTool`, which are exported, not registered |
| [`browser/`](./browser/) | `BrowserTool` (TinyComputer browser members and tasks), `BrowserOpenTool` (simple TinyComputer navigation), `ImageInfoTool` | Both browser tools need `browser.enabled` and the `modules` feature, and are deferred until `tool_search`; `ImageInfoTool` remains direct. Browser navigation uses the shared `http_request.allowed_domains` policy, and the checksum-pinned TinyComputer release module. Consequential actions go through the forced approval gate; `browser_unattended.rs` lets trusted cron/background/approval-free workflow turns skip it for the kinds listed in `[browser] unattended_actions`. |
| [`network/`](./network/) | `HttpRequestTool`, `WebFetchTool`, `CurlTool`, `GitbooksSearchTool`, `GitbooksGetPageTool`, `GmailUnsubscribeTool`, and under the `mcp` Cargo feature `McpListServersTool`, `McpListToolsTool`, `McpCallTool`; `url_guard.rs` is the shared SSRF/allowlist validator, not a tool | `http_request`, `web_fetch`, `curl`, `gmail_unsubscribe` always registered; `gitbooks_*` need `gitbooks.enabled` and, when the active profile sets an `mcp_allowlist`, an entry named `gitbooks`; `mcp_list_servers`/`mcp_list_tools`/`mcp_call_tool` register only when the static `[[mcp_client.servers]]` registry (after the profile allowlist) is non-empty |
| [`document/`](./document/) (feature `documents`) | `DocumentTool` (`generate_document`) | always registered when compiled. Builds `.docx` bytes via the pure-Rust `engine` module (`docx-rs`, no subprocess) inside `spawn_blocking` + `tokio::time::timeout`; allocates the artifact with `agent::artifacts::create_artifact` (kind `Document`), persists `args.json` next to it for the failed-card Retry path, then `finalize_artifact` or `fail_artifact` |
| [`presentation/`](./presentation/) (feature `documents`) | `PresentationTool` (`generate_presentation`) | always registered when compiled. Same shape as `document/` with `ppt-rs` producing `.pptx` and kind `Presentation` |
| [`meta/`](./meta/) | `deferred` (`strip_deferred_from_visible`, `deferred_tool_names`, the `TOOL_SEARCH_NAME` belt opt-in constant); `collapse` helpers (`resolve`, `merge_action_schemas`, `args_without_action`, `strictest_permission`, `any_external_effect`, `unknown_action_message`, `CollapsedAction`) | not registered: `deferred` is the host's half of `ToolExposure::Deferred`: the session builder (`agent/session_host/builder/builder_build.rs`) uses it to split a belt into the advertised set and the still-registered deferred set, and the tinyagents harness advertises its intrinsic `tool_search` / `tool_call` bridge over the latter (ranked per `agent/tinyagents/discovery`). The `collapse` helpers are used by `cron/tools/collapsed.rs` and `memory/tools/collapsed.rs` to merge a multi-action tool's per-action schemas and permissions into one entry |

## Notes

- `document/` and `presentation/` are kept structurally parallel (validate
  input, allocate artifact, generate bytes off the async runtime, finalize or
  fail the artifact) so further artifact-producing tools follow the same shape.
- `meta/` sits outside [`system/`](./system/) because it is the model introspecting its own
  tool surface, not a host capability.
- [`system/mod.rs`](./system/mod.rs) (`security_for_tool_context`) and [`filesystem/gate.rs`](./filesystem/gate.rs)
  (`security_scoped_to_root`, behind `FsGate::scoped_to_workspace`) each grant the run's TinyAgents workspace
  descriptor as `action_dir` plus a `ReadWrite` trusted root on a per-call
  clone of `SecurityPolicy`. They must stay in step; `is_always_forbidden` and
  `is_workspace_internal_path` are evaluated before any trusted-root shortcut.

## Further reading

- [Parent module (`tools`)](../README.md)
- [Native tools overview](../../../../../gitbooks/features/native-tools/README.md)
- [Agent harness architecture](../../../../../gitbooks/developing/architecture/agent-harness.md)
- [Approval gate](../../../../../gitbooks/features/approval-gate.md)
- [tinyagents submodule](../../../../../vendor/tinyagents/README.md)
