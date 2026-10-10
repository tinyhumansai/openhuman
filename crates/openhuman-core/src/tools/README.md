# tools

The agent tool layer. This folder assembles the list of tools an agent session
runs against, holds the cross-cutting built-in tools (filesystem gate,
browser, process and system, generic network, document and presentation
generation), and owns the policy around tools: which ones a channel may see,
which ones are packed out of the prompt, how long a call may run, and how a
failure is described. The agent session builder is the main caller. A small
allowlist of tool-like operations is also exposed over JSON-RPC for the
desktop shell.

Domain-owned tools (memory, cron, flows, wallet, Composio, skills, voice,
agent sub-dispatch) live in their own domains. [`mod.rs`](./mod.rs) re-exports them, so a
single `crate::tools::*` import reaches the full set.

## How it works

### Building a session's registry

`ops::all_tools_with_runtime` ([`ops.rs`](./ops.rs)) is the one place a full registry is
assembled. It is called by the session builder
(`agent/session_host/builder/factory.rs`) and at channel startup
(`channels/runtime/startup/start_channels.rs`). The order of work:

```text
 Config, SecurityPolicy, AuditLogger, RuntimeAdapter, action_dir
        |
        v
 host `node` / `python3` commands used through shell
        |
        v
 base vec: shell, file_read, file_write, grep, glob, list, edit,
           apply_patch, csv_export, sub-agent dispatch, todo, plan,
           cron, time, detect/install, retrieve_tool_output, ...
        |
        v
 feature- and config-gated families appended
   flows, web3, voice, skills, mcp, documents, media, hosting,
   memory (memory::engine::is_on), browser (browser.enabled),
   gitbooks (gitbooks.enabled), search, integrations, composio,
   lsp (OPENHUMAN_LSP_ENABLED), delegate
        |
        v
 post-filter 1: DomainSet      drop tools whose DomainGroup is off
 post-filter 2: ToolGroups     drop tools in a group set to Off
        |
        v
 append use_skill (toolpacks)
        |
        v
 Vec<Box<dyn Tool>>  --> factory.rs: filter_tools_by_user_preference
```

Most families gate twice: once at compile time with a Cargo feature
(`flows`, `web3`, `voice`, `skills`, `mcp`, `modules`, `documents`, `media`,
`hosting`) and once at runtime from config. A tool that cannot
work is left out rather than registered and failing, because a model retries
a failing tool. For example, the `memory` tool registers only while a memory
engine is usable, gitbooks and the static MCP bridge are skipped when their
client cannot be built, and hosting tools register only once a credential
resolves.

The two post-filters both default open: with no ambient `CoreContext` (unit
tests, pre-boot) nothing is dropped. `tool_group` in `ops.rs` maps a tool name
to its `DomainGroup`, by prefix for families that have one (`wallet_`,
`web3_`, `x402_`, `media_`, `mcp_`) and by explicit list for the rest
(skills, flows, voice). A tool name missing from a list falls through to
`Platform` and stays callable when its family is gated off, so the lists must
track the registrations.

`default_tools` and `default_tools_with_runtime` build a minimal registry
(`shell`, `file_read`, `file_write`) with a disabled audit logger. They are
for tests and the lightweight CLI surface.

### Orchestrator tools

The orchestrator agent does not get one generic `spawn_subagent` mega-tool.
`orchestrator_tools::collect_orchestrator_tools` synthesizes one named
`delegate_*` tool per `SubagentEntry::AgentId` in the orchestrator's
`[subagents]` allowlist, with its description taken live from the target's
`AgentDefinition::when_to_use`. A `SubagentEntry::Skills` wildcard expands to
one `ToolExposure::Deferred` `ComposioActionTool` per action of every
connected Composio toolkit. Deferred tools never reach the wire: an agent that
opted into discovery finds them through the harness's `tool_search` bridge and
calls them by name. Connected MCP server actions are registered the same way
by `mcp/registry/action_tool.rs`.

### What the model sees

Several layers narrow the registry before a schema is sent:

- [`user_filter.rs`](./user_filter.rs) maps UI toggle ids to tool names (`TOOL_FAMILIES`). A
  default-on family missing from a stale snapshot is kept; a default-off
  family (installers, mutators, service lifecycle) is stripped unless the user
  enabled it. Tools no family covers are always kept.
- [`toolpacks/`](./toolpacks/) keeps a pack's tools constructed but unadvertised. The agent
  sees `use_skill` and a short pack index instead of the real schemas. See
  [toolpacks/README.md](toolpacks/README.md).
- `ToolExposure::Deferred` tools ([`impl/meta/deferred.rs`](./impl/meta/deferred.rs) holds the host
  half) are left off the wire until found with `tool_search`.
- [`agent_policy/`](./agent_policy/) classifies every tool against the channel's permission
  ceiling into allow, require-approval, deny or hide, and renders the
  prompt's tool-boundary section. See [agent_policy/README.md](agent_policy/README.md).

### Executing a call

The `Tool` trait and its vocabulary come from `tinytools`. The tinyagents
harness runs the loop; OpenHuman's host adapters decide policy. Before a tool
runs, the security gate (`agent/tinyagents/host/security_gate.rs`) checks the
tier, the command class and the approval gate; the tool itself then checks
`SecurityPolicy` for paths, commands and the action budget. The autonomy
policy is off by default (see [`../security/README.md`](../security/README.md)),
so on a default install the path floor and the sandbox do the bounding.
Process-spawning tools in sandboxed mode route through
[`../sandbox/`](../sandbox/README.md). Each call is bounded by [`timeout/`](./timeout/), and
a failure is classified by [`status/`](./status/) into a plain-language cause and next
step.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Export hub: submodules, re-exports of built-in and domain tools, the `tinytools` vocabulary, and the `all_tools_*` controller pair. |
| [`ops.rs`](./ops.rs) | Registry assembly (`default_tools*`, `all_tools*`), all config and feature gating, and `tool_group`. |
| [`orchestrator_tools.rs`](./orchestrator_tools.rs) | Per-subagent `delegate_*` tools and deferred Composio action expansion. |
| [`user_filter.rs`](./user_filter.rs) | `filter_tools_by_user_preference` and the UI toggle to tool-name map. |
| [`host_extensions.rs`](./host_extensions.rs) | Readers over `tinytools`' erased host-extension slots: `pack_registry_handle`, `delegation_target`, `tool_call_id`. |
| [`schemas.rs`](./schemas.rs), [`schemas/`](./schemas/) | The `tools.*` controllers: `registry.rs` (schemas and dispatch), `composio.rs`, `web_search.rs`. |
| [`impl/`](impl/README.md) | Built-in tool families: `filesystem/` (the `FsGate` adapter only; the tools are `tinytools_std::filesystem`), `browser/`, `system/`, `network/`, `meta/`, and the `documents`-gated `document/` and `presentation/`. |
| [`toolpacks/`](toolpacks/README.md) | On-demand tool disclosure (`use_skill`, pack catalog, guides) and `ToolGroups` / `GroupMode`. |
| [`agent_policy/`](agent_policy/README.md) | Per-session tool boundary against a channel's permission ceiling. |
| [`registry/`](registry/README.md) | Read-only discovery registry across MCP stdio, controller and connected MCP tools, plus policy diagnostics. |
| [`timeout/`](timeout/README.md) | Process-wide tool timeout and process-group kill helpers. |
| [`status/`](./status/) | Tool-call lifecycle state and failure classification (`classify`, `describe`, `tool_execution_error`). |

Where the built-in tools come from:

| Tools | Source |
| --- | --- |
| `file_read`, `file_write`, `edit`, `apply_patch`, `grep`, `glob`, `list`, `read_diff`, `csv_export`, `git_operations`, `run_linter`, `run_tests`, `image_info`, `read_workspace_state` | `tinytools_std::filesystem`, gated by [`impl/filesystem/gate.rs`](./impl/filesystem/gate.rs) |
| `detect_tools`, `curl`, `pushover` | `tinytools_std` |
| `http_request`, `web_fetch` | `tinytools_std::network`, wired with host limits, TinyJuice extraction and the x402 handler in [`impl/network/host.rs`](./impl/network/host.rs) |
| `current_time`, `resolve_time`, `ask_user_clarification`, `wait`, `wait_loop` | `tinyagents_harness::tools` |
| `shell`, `install_tool`, `schedule`, `proxy_config`, `lsp`, `update_check`, `update_apply`, `retrieve_tool_output` | [`impl/system/`](./impl/system/) |
| `gitbooks_search`, `gitbooks_get_page`, `gmail_unsubscribe`, `mcp_list_servers`, `mcp_list_tools`, `mcp_call_tool`, `mcp_<server>_<tool>` | [`impl/network/`](./impl/network/) (MCP tools behind `mcp`) |
| `browser`, `browser_open` | [`impl/browser/`](./impl/browser/) (behind `modules`) |
| `generate_document`, `generate_presentation` | [`impl/document/`](./impl/document/), [`impl/presentation/`](./impl/presentation/) (behind `documents`) |
| `web_search` and the other search roles | `crate::search::build_search_tools` |

## Key types and entry points

- `Tool`, `ToolSpec`, `ToolResult`, `ToolExposure`, `PermissionLevel`,
  `ToolScope`, `ToolCategory`: re-exported from `tinytools` in `mod.rs` so an
  embedder implementing a tool reaches the vendored copy.
- `ops::all_tools_with_runtime`: the full registry. Its last parameter,
  `approval_workspace_root`, scopes `file_write`'s approval prompts.
- `orchestrator_tools::collect_orchestrator_tools` and
  `collect_deferred_integration_actions`.
- `filter_tools_by_user_preference` (crate-internal, `user_filter.rs`).
- `toolpacks::{append_pack_tools, bind_pack_registry,
  strip_packed_from_visible}` and `toolpacks::groups::{ToolGroups,
  GroupMode, current}`. `openhuman-embed` re-exports `ToolGroups` and
  `GroupMode` as its tool-visibility control.
- `timeout::{tool_execution_timeout_secs, set_tool_timeout_secs,
  output_or_kill, kill_process_group}`.
- `status::{classify, ToolLifecycleState, ToolFailureClass,
  ClassifiedFailure}`.

## RPC surface

Namespace `tools`, registered in `core/all.rs` through
`all_tools_registered_controllers`. Wire names follow
`openhuman.<namespace>_<function>`. The list is deliberately small; anything
else is agent-only.

| Method | Purpose |
| --- | --- |
| `openhuman.tools_composio_execute` | Run a Composio action through the mode-aware client (backend-proxied or direct). |
| `openhuman.tools_web_search` | Ranked web search through the `search` role; results plus the provider that answered. |
| `openhuman.tools_web_answer` | Grounded answer with citations through the `answer` role. |
| `openhuman.tools_web_contents` | Page contents for given URLs through the `contents` role. |
| `openhuman.tools_searxng_search` | The `search` role pinned to a self-hosted SearXNG (requires SearXNG enabled). |

[`registry/`](./registry/) registers a second namespace, `tool_registry` (`list`, `get`,
`diagnostics`). Dotted ids such as `tools.web_search` in that registry are
`tool_id`s, not RPC methods.

## Boundaries

- The `Tool` trait, `ToolResult`, `ToolExposure` and the generic filesystem
  and network tools belong to `tinytools` (`vendor/tinyagents/vendor/tinytools`).
  Use that one copy; a second path produces incompatible types.
- The agent loop, tool-call parsing, `tool_search`, `use_skill` and the pack
  registry handle belong to `tinyagents` (`vendor/tinyagents`).
- Execution policy (paths, commands, approvals) is `security/`; where a
  command runs is `sandbox/`.
- Search provider resolution and the TinySearch bridge are `crate::search`
  (see [`../search/README.md`](../search/README.md)); the module itself is
  `tinysearch`.
- New domain tools go in the owning domain's `tools.rs` and are re-exported
  through `mod.rs`. Only cross-cutting families belong under [`impl/`](./impl/).
- This folder has no persistence and no event-bus subscriber. Approval
  routing reads `Tool::external_effect_with_args`; the gate itself is
  `security/approval/`.

## Gotchas

- Override `external_effect_with_args`, not only `external_effect`, when a
  tool's effect depends on its arguments (Composio `execute` versus `list`).
  The harness checks the per-call variant.
- `PermissionLevel` ordering is compared with `<` against a channel's maximum.
  A multi-action tool should return its lowest level from
  `permission_level()` and do the per-call check in
  `permission_level_with_args`.
- `is_concurrency_safe` tells the harness a call may run alongside others in
  the same batch. Return `true` only for calls with no shared side effects.
- The shell inherits the host `PATH`; Node.js and Python commands run with the host toolchain.
- The browser shares `http_request.allowed_domains` but strips the `*`
  wildcard, so unifying the lists can only narrow browser reach. Allow-all
  stays behind `OPENHUMAN_BROWSER_ALLOW_ALL`.
- Search is configured per provider and per role in `[search]`; disabling all
  providers removes the search tools. User-facing setup is in
  [`gitbooks/features/native-tools/web-search.md`](../../../../gitbooks/features/native-tools/web-search.md).

## Tests

Registry tests are split by concern under `ops_tests*.rs` (default registry,
capability gating, domain families, Composio registration, REPL tools,
execution and serde). Every other module has a sibling `*_tests.rs`. Run with
`cargo test -p openhuman tools::` or `pnpm debug rust tools`.

## Further reading

- [Native tools overview](../../../../gitbooks/features/native-tools/README.md)
- [Agent harness architecture](../../../../gitbooks/developing/architecture/agent-harness.md)
- [Approval gate](../../../../gitbooks/features/approval-gate.md)
- [tinyagents submodule](../../../../vendor/tinyagents/README.md)
