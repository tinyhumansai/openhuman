# runtime/node

Two unrelated things share this directory, and the difference is the first thing
to understand about it.

**The toolchain client** asks the `tinyruntime` module for a Node.js toolchain
and adapts the answer onto `ResolvedNode`, so `node_exec`, `npm_exec`, `shell`,
and Node-dependent skills have a trusted `node`/`npm` on a stable path.

**The tool bridge** exposes the full agent tool registry over JSON-RPC under
`javascript.*`, so an embedded JS host can enumerate and run tools by name. It
has nothing to do with Node beyond being reachable from JavaScript, which is why
it is not gated with the rest.

The public-facing language slot is the sibling [`javascript`](../javascript/)
module, which re-exports this module's surface under `javascript`-prefixed names.

## What moved out

Everything that used to make this the largest module in `runtime/`: system-node
probing, distribution selection, `SHASUMS256.txt` fetching, streaming download
with SHA-256 verification, `.tar.xz` / `.zip` extraction, atomic install, cache
roots, and the guards against a workspace-vendored fake install tree.

That is all in the `tinyruntime` module now, where one implementation serves
every language, and it is reached through
[`modules::runtime`](../../modules/runtime.rs). The visible consequence for this
repository is that `xz2` and its static liblzma C build left the manifest
entirely — the first native toolchain build removed rather than merely gated.

## Responsibilities

- Ask the module to resolve a Node toolchain, installing one when the host has
  none, and adapt the reply onto `ResolvedNode` (`node_bin`, `npm_bin`,
  `bin_dir`, `version`, `source`).
- Memoise that answer locally so `try_cached()` can answer **without awaiting** —
  the shell consults it on every command to decide whether to prepend a managed
  `bin/` directory to `PATH`, and a blocking call there would make every
  unrelated command wait on a bus round trip.
- Build the full agent tool registry on demand and expose two RPC controllers:
  list tool metadata, and execute a named tool returning an MCP-style
  `ToolResult`.
- Publish `ToolExecutionStarted` / `ToolExecutionCompleted` around bridge tool
  execution.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused: submodule decls, the `runtime-node` gate, and `pub use` re-exports including the controller registry pair. |
| `bootstrap.rs` | The toolchain client. `NodeBootstrap` (`resolve`, `probe_installed`, `try_cached`), `ResolvedNode`, `NodeSource`. Adapts a module `ResolvedRuntime`; derives `npm` when the provider does not report it. |
| `stub.rs` | Type surface for `runtime-node`-less builds. `try_cached`/`probe_installed` return `None`; `resolve` errors with a build fact. |
| `ops.rs` | Bridge logic: `build_runtime_tools`, `list_tools`, `execute_tool` (event publish + timing). |
| `rpc.rs` | RPC param structs and `*_handler` fns; loads config and delegates through the `javascript` alias. |
| `schemas.rs` | Controller schemas + registered controllers for `javascript_list_tools` / `javascript_execute_tool`. |
| `types.rs` | `RuntimeToolSummary`, `ExecuteToolOutcome` serde types. |

## Public surface

- Client: `NodeBootstrap`, `NodeSource`, `ResolvedNode`.
- Bridge ops: `execute_tool`, `list_tools`.
- Types: `RuntimeToolSummary`, `ExecuteToolOutcome` (via `types`).
- Controller registry pair: `all_runtime_node_controller_schemas`,
  `all_runtime_node_registered_controllers`.

## RPC / controllers

Registered under namespace `javascript` (schemas wired into `src/core/all.rs`
via the `javascript` module's `all_javascript_*` aliases, not under a
`runtime_node` name):

| Method | Inputs | Output |
| --- | --- | --- |
| `javascript.list_tools` | none | `tools`: array of tool metadata. |
| `javascript.execute_tool` | `tool_name` (required), `args` (optional, defaults `{}`), `prefer_markdown` (optional bool) | `tool_name`, `elapsed_ms`, `result`. |

Unknown tool name → error ``unknown tool `<name>` ``.

## Agent tools

This module owns **no** tools of its own. It builds the *entire* agent tool
registry on demand (`tools::all_tools_with_runtime`) to back the bridge. The
`node_exec`, `npm_exec`, and `shell` tools live in
`src/openhuman/tools/impl/system/` and consume this module's `NodeBootstrap`.

## Events

`ops::execute_tool` publishes around each bridge invocation, with
`session_id = "javascript"`:

- `DomainEvent::ToolExecutionStarted`
- `DomainEvent::ToolExecutionCompleted` (with `success`, `elapsed_ms`)

## Persistence

**None here any more.** The managed install cache belongs to the `tinyruntime`
module, which decides where it lives from the settings each request carries.
This module reads `config.node` (`enabled`, `prefer_system`, `version`,
`cache_dir`) only to build those requests.

## Dependencies

- `crate::openhuman::modules::runtime` — the module client this delegates to.
- `crate::openhuman::config` — the settings each request carries.
- `crate::openhuman::tools`, `security`, `agent::host_runtime`, `memory` — the
  registry the bridge enumerates and executes.
- `crate::core::event_bus`, `crate::core::all`, `crate::rpc` — events and RPC
  plumbing.

External crates: `tinyruntime-bus`, `tokio`, `anyhow`, `serde`/`serde_json`,
`tracing`, `async-trait`. No HTTP client, no archive crates, no digest crate —
those went with the machinery.

## Used by

- `src/openhuman/runtime/javascript/mod.rs` — the public language slot.
- `src/openhuman/tools/impl/system/{node_exec,npm_exec,shell}.rs` — hold an
  `Arc<NodeBootstrap>`; the exec tools call `resolve()`, `shell` uses the
  non-blocking `try_cached()`.
- `src/openhuman/agent/harness_init/registry.rs` — the `node_runtime` init step
  uses `probe_installed()` to decide whether provisioning is visible work.
- `src/core/all.rs` — registers the `javascript.*` controllers.

## Notes / gotchas

- **Naming asymmetry**: the directory is `node` but its RPC namespace and public
  aliases are `javascript`. That indirection is what let the backend underneath
  be replaced by a bus module without churning a single caller.
- **`build_runtime_tools` is not cheap**: each bridge call rebuilds the full tool
  registry from `Config`. There is no caching at the bridge layer — the
  memoisation here is only for toolchain resolution.
- **The local cache is not redundant with the module's.** The module memoises
  too, but only this one can answer without awaiting, which is the entire reason
  the shell can inject `PATH` without blocking.
- **Version policy lives in the provider now.** Major-only matching, and the
  `prefer_system = false` escape hatch for strict pinning, are decisions of
  `tinyruntime-nodejs`; this module carries the setting rather than the rule.
- **A toolchain without `npm` still resolves.** `npm_bin` is derived when the
  provider does not report it: refusing would take `node_exec` down along with
  `npm_exec`, for an install that runs `node` perfectly well.
