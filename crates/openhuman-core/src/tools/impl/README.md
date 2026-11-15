# tools/impl

Built-in implementations of the cross-cutting tool families that `tools/ops.rs`
assembles into the agent registry. Ownership rule from `AGENTS.md`: only
cross-cutting capabilities (filesystem, browser, generic process/system, generic
here. Domain-owned tools (memory, cron, wallet, composio, integrations, voice,
agent sub-dispatch, ...) live in their own domain's `tools.rs` and are only
re-exported through `tools/mod.rs`. Do not add a family under `impl/` for a
domain capability.

`tools/mod.rs` mounts this directory as `#[path = "impl/mod.rs"] pub(crate) mod
implementations` (`impl` is a keyword) and glob re-exports it, so tool structs
are imported as `crate::tools::*`. `impl/mod.rs` declares four families
(`system/`, `filesystem/`, `browser/`, `network/`) and has no Cargo-feature
gates of its own; feature gates inside a family are noted below. The
`generate_document` / `generate_presentation` tools are domain-owned and live in
[`artifacts/tools/`](../../artifacts/tools/) behind the `documents` feature,
built by `artifacts::tools::registered_tools`.

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
| [`system/`](./system/) | `ShellTool`, `InstallToolTool`, `DetectToolsTool`, `CurrentTimeTool`, `ResolveTimeTool`, `ProxyConfigTool`, `PushoverTool`, workspace and retrieval helpers. |
| [`filesystem/`](./filesystem/) | Host adapter only: `SecurityPolicy` implements `tinyagents_harness_tools::fs::filesystem::FsGate`. The tools (`FileReadTool`, `FileWriteTool`, `EditFileTool`, `ApplyPatchTool`, `GrepTool`, `GlobTool`, `ListFilesTool`, `ReadDiffTool`, `CsvExportTool`, `GitOperationsTool`, `RunLinterTool`, `RunTestsTool`, `UpdateMemoryMdTool`) live in `tinyagents_harness_tools::fs::filesystem` and are imported directly by `tools/ops.rs` | always registered, except `ReadDiffTool`, `RunLinterTool`, and `RunTestsTool`, which are exported, not registered |
| [`browser/`](./browser/) | `BrowserTool` (TinyComputer browser members and tasks), `BrowserOpenTool` (simple TinyComputer navigation), `ImageInfoTool` | Both browser tools need `browser.enabled` and the `modules` feature, and are deferred until `tool_search`; `ImageInfoTool` remains direct. Browser navigation uses the shared `http_request.allowed_domains` policy, and the checksum-pinned TinyComputer release module. Consequential actions go through the forced approval gate; `browser_unattended.rs` lets trusted cron/background/approval-free workflow turns skip it for the kinds listed in `[browser] unattended_actions`. |
| [`network/`](./network/) | `HttpRequestTool`, `WebFetchTool`, `CurlTool`, `GitbooksSearchTool`, `GitbooksGetPageTool`, and under the `mcp` Cargo feature `McpListServersTool`, `McpListToolsTool`, `McpCallTool`; `url_guard.rs` is the shared SSRF/allowlist validator, not a tool | `http_request`, `web_fetch`, `curl` always registered; `gitbooks_*` need `gitbooks.enabled` and, when the active profile sets an `mcp_allowlist`, an entry named `gitbooks`; `mcp_list_servers`/`mcp_list_tools`/`mcp_call_tool` register only when the static `[[mcp_client.servers]]` registry (after the profile allowlist) is non-empty |

## Notes

- The update tools are owned and registered from [`platform/update/tools.rs`](../../platform/update/tools.rs), alongside the update domain they call.

- [`filesystem/gate.rs`](./filesystem/gate.rs) owns workspace descriptor
  scoping. The filesystem gate and system policy resolver use the same helper,
  which grants the descriptor root as `action_dir` plus a `ReadWrite` trusted
  root on a per-call clone of `SecurityPolicy`; `TinySecurity protected-path policy` and
  `TinySecurity internal-state reservations` are evaluated before any trusted-root shortcut.

## Further reading

- [Parent module (`tools`)](../README.md)
- [Native tools overview](../../../../../docs/gitbooks/en/features/native-tools/README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [tinyagents submodule](../../../../../vendor/tinyagents/README.md)
