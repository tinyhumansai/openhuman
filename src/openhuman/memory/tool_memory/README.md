# memory_tools

Tool-scoped memory: durable rules / learnings keyed per tool name. Distinct
from generic namespace memory and from `learning::tool_tracker` statistics.

## Namespace convention

Each tool gets its own namespace `tool-{tool_name}`. Build the string via the
`tool_memory_namespace` re-export — never hard-code it.

## Layout

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Module root + public re-exports. |
| [`mod.rs`](mod.rs) | Re-exports crate types/store and defines `tool_memory_store(Arc<dyn Memory>)`. |
| [`capture.rs`](capture.rs) | `ToolMemoryCaptureHook` — `PostTurnHook` impl that captures user edicts and repeated tool failures into the store (host-retained). |
| [`prompt.rs`](prompt.rs) | **Shim** — re-exports the crate `ToolMemoryRulesSection` + `render_tool_memory_rules` + `TOOL_MEMORY_HEADING`, and keeps the host `PromptSection` impl that plugs the section into the system-prompt builder. |
| [`tools/`](tools/) | Agent-facing read/write tools: `MemoryToolsListTool` (list rules for a tool), `MemoryToolsPutTool` (upsert a rule). |
| [`test_helpers.rs`](test_helpers.rs) | `#[cfg(test)]` `MockMemory` used by `capture::tests` (the store engine's own coverage lives in the crate). |

## How it fits

The agent harness:
1. **Reads** at session build — `ToolMemoryRulesSection::render` walks every
   `tool-*` namespace and pins Critical/High rules into the system prompt.
2. **Writes** at turn end — `ToolMemoryCaptureHook` parses the user message
   for edicts (`"never do X"`, `"always Y"`, …) and inserts rules.
3. **Direct read/write** — `tools::MemoryTools{List,Put}Tool` let the agent
   itself inspect / record rules mid-session.

## Layer rules

- No upward dependencies — only `memory::Memory` trait (via `Arc<dyn Memory>`)
  and project-wide primitives (`tools::traits::Tool`, `serde_json`).
- `MockMemory` is `#[cfg(test)]`-only — never available outside test builds.
- Re-exports in `mod.rs` are the public surface. Crate-owned type and store
  forwarding files were removed; consumers should use the domain root.
