# javascript

First-class **JavaScript language slot** for the core. This module is a thin re-export facade: it gives the rest of the codebase a stable `crate::openhuman::javascript` surface that talks to a *language* (`javascript`) rather than to a concrete backend. Today the implementation backend is the managed Node.js runtime in `crate::openhuman::runtime_node`; the facade exists so a future sibling slot (`python`, `ruby`, or an alternate JS backend) can swap in without churning callers. It owns no logic of its own — every symbol it exposes is a `pub use` of a `runtime_node` item.

## Responsibilities

- Provide the canonical `openhuman::javascript::*` import path for Node.js runtime resolution/bootstrap and the agent-tool bridge.
- Re-export the JS-namespaced RPC controller schemas/registry (`javascript.list_tools`, `javascript.execute_tool`) under stable `all_javascript_*` names so `src/core/all.rs` can wire them.
- Re-export `NodeBootstrap` and related resolve/download/extract types used by the system exec tools to locate a `node` binary.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/javascript/mod.rs` | Entire module. Module docstring + `pub use` re-exports from `runtime_node`. No types, no logic, no tests. |

> All behavior lives in `src/openhuman/runtime_node/` (`bootstrap.rs`, `downloader.rs`, `extractor.rs`, `resolver.rs`, `ops.rs`, `schemas.rs`, `rpc.rs`, `types.rs`). This facade only renames/forwards their public surface.

## Public surface

Re-exported from `runtime_node`:

- **Types**: `ExecuteToolOutcome`, `RuntimeToolSummary` (from `runtime_node::types`); `NodeBootstrap`, `NodeSource`, `ResolvedNode`, `NodeDistribution`, `SystemNode`.
- **Functions**: `execute_tool`, `list_tools` (tool bridge); `detect_system_node`, `parse_node_version` (system node detection); `download_distribution`, `fetch_shasums`, `atomic_install`, `extract_distribution` (managed-toolchain install path).
- **Controller wiring**: `all_javascript_controller_schemas` (← `all_runtime_node_controller_schemas`), `all_javascript_registered_controllers` (← `all_runtime_node_registered_controllers`).

## RPC / controllers

Namespace `javascript` (defined in `runtime_node/schemas.rs`, surfaced through this facade):

| Method | Inputs | Outputs |
| --- | --- | --- |
| `javascript.list_tools` | — | `tools`: array of tool metadata (`name`, `description`, `category`, `permission_level`, `scope`, `supports_markdown`, `parameters`). |
| `javascript.execute_tool` | `tool_name` (required), `args` (optional Json, defaults to `{}`), `prefer_markdown` (optional bool) | `tool_name`, `elapsed_ms` (u64), `result` (MCP-style ToolResult: `{content, is_error, markdownFormatted?}`). |

Handlers (`runtime_node/rpc.rs`) load config via `config::rpc::load_config_with_timeout`, build the full tool set via `tools::all_tools_with_runtime`, then list or dispatch a tool by exact name. Results are returned as `RpcOutcome` (CLI-compatible JSON).

## Agent tools

This module owns no `tools.rs`. Instead it acts as a **bridge** to the cross-cutting tool registry: `runtime_node::ops::build_runtime_tools` constructs the full tool set (`tools::all_tools_with_runtime`) and `list_tools` / `execute_tool` enumerate or invoke them by name.

## Events

`execute_tool` (in `runtime_node/ops.rs`) publishes via `publish_global`:

- `DomainEvent::ToolExecutionStarted { tool_name, session_id: "javascript" }`
- `DomainEvent::ToolExecutionCompleted { tool_name, session_id: "javascript", success, elapsed_ms }`

No event subscribers (`bus.rs`) are owned here.

## Persistence

No persisted domain state (`store.rs`). The managed-Node install path (`extractor::atomic_install`) writes a Node toolchain to disk, but that is filesystem installation, not domain state.

## Dependencies

(Direct dependency of this facade module: `runtime_node`. Transitive deps below come from the `runtime_node` implementation it forwards to.)

- `crate::openhuman::runtime_node` — the entire backing implementation; all re-exports come from here.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) + `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) — RPC controller schema contract.
- `crate::core::event_bus` — publishes `ToolExecutionStarted` / `ToolExecutionCompleted`.
- `crate::openhuman::config` — loads `Config` to build the tool set; RPC handlers use `config::rpc::load_config_with_timeout`.
- `crate::openhuman::tools` — `all_tools_with_runtime`, `Tool`, `ToolCallOptions`, `ToolScope`; the actual tool registry being bridged.
- `crate::openhuman::agent::host_runtime` — `NativeRuntime` / `RuntimeAdapter` injected when building tools.
- `crate::openhuman::memory` / `memory_store` — `create_memory_with_local_ai` for memory-backed tools.
- `crate::openhuman::security` — `SecurityPolicy::from_config` + workspace audit logger.
- `crate::openhuman::skills::types::ToolResult` — tool result envelope type.

## Used by

- `src/core/all.rs` — wires `all_javascript_registered_controllers` / `all_javascript_controller_schemas` into the controller registry; the about-app catalog describes namespace `javascript`.
- `src/openhuman/tools/ops.rs`, `tools/impl/system/shell.rs`, `node_exec.rs`, `npm_exec.rs` — import `openhuman::javascript::NodeBootstrap` for Node binary resolution.
- `src/openhuman/runtime_node/rpc.rs` — calls `javascript::list_tools` through the facade.

## Notes / gotchas

- This is a **rename-only facade** (see the canonical "thin facade" exception in CLAUDE.md): no `types.rs`/`ops.rs`/`store.rs` here by design. Edit behavior in `runtime_node`, not here.
- `javascript.execute_tool` rebuilds the entire tool set on every call (via `build_runtime_tools`), then finds the named tool — there is no persistent tool cache.
- The `session_id` on emitted tool events is the literal string `"javascript"`, not a real chat/session id.
- Tool-list output is sorted by tool name (`list_tools` in `runtime_node/ops.rs`).
- Both RPC handlers are async and resolve config through `load_config_with_timeout`, so a slow/blocked config load surfaces as an RPC error.
