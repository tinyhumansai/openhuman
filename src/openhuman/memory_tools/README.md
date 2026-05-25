# memory_tools

Tool-scoped memory: durable rules / learnings keyed per tool name. Distinct
from generic namespace memory and from `learning::tool_tracker` statistics.

## Namespace convention

Each tool gets its own namespace `tool-{tool_name}`. Build the string via
[`types::tool_memory_namespace`] — never hard-code it.

## Layout

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Module root + public re-exports. |
| [`types.rs`](types.rs) | `ToolMemoryRule` (id, tool_name, rule text, priority, source, tags, created_at, updated_at) + `ToolMemoryPriority` (Normal / High / Critical) + `ToolMemorySource` (UserExplicit / PostTurn / Programmatic) + `tool_memory_namespace(tool_name)`. |
| [`store.rs`](store.rs) | `ToolMemoryStore` over `Arc<dyn Memory>`: `put_rule`, `get_rule`, `list_rules`, `delete_rule`, `rules_for_prompt`, `list_tool_names`, `record`, `list_rules_json`. |
| [`store_tests.rs`](store_tests.rs) | Store coverage against the `MockMemory` from `test_helpers`. |
| [`capture.rs`](capture.rs) | `ToolMemoryCaptureHook` — `PostTurnHook` impl that captures user edicts and repeated tool failures into the store. |
| [`prompt.rs`](prompt.rs) | `ToolMemoryRulesSection` + `render_tool_memory_rules` — prompt section that pins Critical / High rules into the system prompt so they survive compression. `TOOL_MEMORY_HEADING` + `TOOL_MEMORY_PROMPT_CAP` constants. |
| [`tools/`](tools/) | Agent-facing read/write tools: `MemoryToolsListTool` (list rules for a tool), `MemoryToolsPutTool` (upsert a rule). |
| [`test_helpers.rs`](test_helpers.rs) | `#[cfg(test)]` `MockMemory` used by `store_tests` + `capture::tests`. |

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
- Re-exports in `mod.rs` are the public surface; the underlying submodules
  are `pub` so test code can reach in but consumers should go through the
  re-exports.
