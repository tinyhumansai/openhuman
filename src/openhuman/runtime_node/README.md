# runtime_node

Managed **Node.js runtime** for the core, plus a thin **tool bridge** that lists and dispatches agent-callable tools through a `javascript` RPC namespace. The runtime half resolves or installs a pinned Node.js toolchain (reusing a compatible host `node` when present, otherwise downloading + SHA-256-verifying + extracting an official distribution from nodejs.org) so that `node_exec` / `npm_exec` / `shell` tools and Node-dependent skills have a trusted `node`/`npm` on a stable path. The bridge half exposes the full agent tool registry over JSON-RPC under `javascript.*` so callers (e.g. an embedded JS host) can enumerate and run tools by name. The public-facing language slot is the sibling [`javascript`](../javascript/) module, which re-exports this module's surface under `javascript`-prefixed names.

## Responsibilities

- Detect a compatible system `node` on `PATH` (major-version match) and verify `npm` is also usable before reusing it.
- Resolve/install a managed Node.js toolchain when no compatible system node exists: pick the host archive, fetch `SHASUMS256.txt`, download with streaming SHA-256 verification, extract (`.tar.xz` / `.zip`), and atomically install into a user-owned cache root.
- Memoise the resolved toolchain behind a `tokio::sync::Mutex` so concurrent callers never race the download/extract/install pipeline; offer a non-blocking `try_cached()` peek for transparent `PATH` injection.
- Build the full agent tool registry on demand and expose two RPC controllers: list tool metadata, and execute a named tool returning an MCP-style `ToolResult`.
- Publish `ToolExecutionStarted` / `ToolExecutionCompleted` domain events around bridge tool execution.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/runtime_node/mod.rs` | Export-focused: submodule decls + `pub use` re-exports, including `all_runtime_node_controller_schemas` / `all_runtime_node_registered_controllers`. |
| `src/openhuman/runtime_node/resolver.rs` | Synchronous system-node probe. `detect_system_node`, `parse_node_version`, `SystemNode`. `PATH` walk with execute-bit filtering, `node --version` / `npm --version` probes with a 5s timeout. Major-version match only. |
| `src/openhuman/runtime_node/bootstrap.rs` | Orchestrator. `NodeBootstrap` (serialised + memoised `resolve()`, `try_cached()`), `ResolvedNode`, `NodeSource`. Picks system vs managed, computes the cache root (user cache by default, never workspace-local unless forced), guards against cache-root escape / spoofed installs via canonicalised `starts_with`. |
| `src/openhuman/runtime_node/downloader.rs` | `NodeDistribution` (host triple → archive name/URL), `fetch_shasums`, `download_distribution`. Streams to disk while hashing; **mandatory** SHA-256 match or the partial file is deleted. |
| `src/openhuman/runtime_node/extractor.rs` | `extract_distribution` (`.tar.xz` via `xz2`+`tar`, `.zip` via `zip`, both in `spawn_blocking`), `atomic_install` (rename into place with backup/restore). Asserts a single top-level folder per archive. |
| `src/openhuman/runtime_node/ops.rs` | Bridge logic: `build_runtime_tools` (assembles `SecurityPolicy`, audit logger, `NativeRuntime`, `Memory`, then `tools::all_tools_with_runtime`), `list_tools`, `execute_tool` (event publish + timing). |
| `src/openhuman/runtime_node/rpc.rs` | RPC param structs (`ListToolsParams`, `ExecuteToolParams`) and `*_handler` fns; loads config via `config::rpc` and delegates through the `javascript` alias, wrapping results in `RpcOutcome`. |
| `src/openhuman/runtime_node/schemas.rs` | Controller schemas + registered controllers for `javascript_list_tools` / `javascript_execute_tool`; `handle_*` deserialise params and call `rpc.rs`. |
| `src/openhuman/runtime_node/types.rs` | `RuntimeToolSummary`, `ExecuteToolOutcome` serde types. |

## Public surface

From `mod.rs` re-exports:

- Bootstrap: `NodeBootstrap`, `NodeSource`, `ResolvedNode`.
- Downloader: `download_distribution`, `fetch_shasums`, `NodeDistribution`.
- Extractor: `atomic_install`, `extract_distribution`.
- Resolver: `detect_system_node`, `parse_node_version`, `SystemNode`.
- Bridge ops: `execute_tool`, `list_tools`.
- Types: `RuntimeToolSummary`, `ExecuteToolOutcome` (via `types`).
- Controller registry pair: `all_runtime_node_controller_schemas`, `all_runtime_node_registered_controllers`.

## RPC / controllers

Registered under namespace `javascript` (schemas wired into `src/core/all.rs` via the `javascript` module's `all_javascript_*` aliases, not under a `runtime_node` name):

| Method | Inputs | Output |
| --- | --- | --- |
| `javascript.list_tools` | none | `tools`: array of tool metadata (name, description, category, permission_level, scope, supports_markdown, parameters). |
| `javascript.execute_tool` | `tool_name` (required), `args` (optional, defaults `{}`), `prefer_markdown` (optional bool) | `tool_name`, `elapsed_ms`, `result` (MCP-style `ToolResult`: `{content, is_error, markdownFormatted?}`). |

Handlers load config via `config::rpc::load_config_with_timeout`, return `RpcOutcome` (`into_cli_compatible_json`). Unknown tool name → error `unknown tool \`<name>\``.

## Agent tools

This module owns **no** tools of its own. Instead it builds the *entire* agent tool registry on demand (`tools::all_tools_with_runtime`) to back the `javascript.execute_tool` / `javascript.list_tools` bridge. The actual `node_exec`, `npm_exec`, and `shell` tools live in `src/openhuman/tools/impl/system/` and consume this module's `NodeBootstrap` for binary resolution / `PATH` injection.

## Events

`ops::execute_tool` publishes (via `core::event_bus::publish_global`) around each bridge invocation, with `session_id = "javascript"`:

- `DomainEvent::ToolExecutionStarted`
- `DomainEvent::ToolExecutionCompleted` (with `success`, `elapsed_ms`)

No event-bus subscribers (`bus.rs`) are defined.

## Persistence

No domain `store.rs`. The only on-disk state is the **managed Node.js install cache**, resolved by `NodeBootstrap::cache_root()` (first hit wins):

1. Explicit `config.node.cache_dir` (honoured verbatim).
2. `dirs::cache_dir()/openhuman/node-runtime` — the default, user-owned.
3. `{workspace}/node-runtime/` — last-resort fallback (warned; less secure).

Reads `NodeConfig` (`config.node`): `enabled`, `prefer_system`, `version`, `cache_dir` (env overrides like `OPENHUMAN_NODE_ENABLED`, `OPENHUMAN_NODE_VERSION` in config loader).

## Dependencies

- `crate::openhuman::config` (`Config`, `schema::NodeConfig`, `rpc`) — runtime config, version/cache settings, RPC config loading.
- `crate::openhuman::tools` (`Tool`, `ToolCallOptions`, `ToolScope`, `all_tools_with_runtime`) — the registry the bridge enumerates/executes.
- `crate::openhuman::security` (`SecurityPolicy`, workspace audit logger) — built per `build_runtime_tools` call to scope tool capability.
- `crate::openhuman::agent::host_runtime` (`NativeRuntime`, `RuntimeAdapter`) — runtime adapter injected into tool construction.
- `crate::openhuman::memory` / `memory_store` (`Memory`, `create_memory_with_local_ai`) — memory backend wired into memory-aware tools.
- `crate::openhuman::skills::types::ToolResult` — result envelope returned by `execute_tool`.
- `crate::openhuman::javascript` — the language-slot alias module the `rpc.rs` handlers call through (`list_tools` / `execute_tool`).
- `crate::core::event_bus` (`publish_global`, `DomainEvent`) — tool execution events.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) + `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) + `crate::rpc::RpcOutcome` — RPC controller plumbing.

External crates: `reqwest`, `sha2`, `hex`, `xz2`, `tar`, `zip`, `tokio`, `wait_timeout`, `dirs`, `anyhow`, `serde`/`serde_json`, `tracing`, `async-trait`.

## Used by

- `src/openhuman/javascript/mod.rs` — re-exports this entire surface under `javascript`-prefixed names (the public language slot).
- `src/openhuman/tools/impl/system/{node_exec,npm_exec,shell}.rs` — hold an `Arc<NodeBootstrap>`; `node_exec`/`npm_exec` call `resolve()`, `shell` uses non-blocking `try_cached()` for transparent `PATH` injection.
- `src/openhuman/tools/ops.rs` and `src/openhuman/agent/tools/delegate_to_personality.rs` — reference the bootstrap/runtime surface.
- `src/openhuman/runtime_python/bootstrap.rs` — a sibling runtime modeled on the same pattern.
- `src/core/all.rs` — registers the `javascript.*` controllers via the `javascript` aliases.

## Notes / gotchas

- **Naming asymmetry**: the module is `runtime_node` but its RPC namespace and public aliases are `javascript`. The `javascript` module is a deliberate language-slot indirection so a future backend (or `python`/`ruby`) can swap in without churning callers.
- **`build_runtime_tools` is not cheap**: each `list_tools`/`execute_tool` call rebuilds the full tool registry (security policy, audit logger, memory backend) from `Config`. There is no caching at the bridge layer — the memoisation in `bootstrap.rs` is only for Node toolchain resolution, not for tool construction.
- **Integrity is load-bearing, no opt-out**: downloads must match the official `SHASUMS256.txt` digest or the archive is deleted and the call fails. `probe_managed_install` canonicalises and requires the install to live under the resolved cache root to defeat a workspace-vendored fake `node-v*/` tree (PR #723 finding).
- **System-node reuse requires npm**: a compatible `node` with a missing/broken `npm` is rejected so the managed path can supply a complete toolchain (distros that split `nodejs`/`npm`).
- **Version match is major-only** (`parse_node_version`); point releases are accepted. Set `node.prefer_system = false` for strict pinning, or `node.enabled = false` to disable the runtime entirely (then `resolve()` bails).
- **Self-healing cache**: a managed install missing `npm` (e.g. download interrupted after `node` extracted) is treated as unusable and reinstalled rather than reused forever.
