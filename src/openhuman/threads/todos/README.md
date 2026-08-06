# todos

Compatibility surface for OpenHuman task-board callers.

The task-board model and behavior are owned by
`tinyagents::graph::todos`: types, normalization, markdown rendering, CRUD,
plan decisions, session links, the single-`in_progress` invariant, atomic
claims, durable storage, and the in-memory scratch board.

OpenHuman keeps this module to preserve app-specific integration:

- `ops.rs` maps `BoardLocation` onto TinyAgents stores, preserves the optional
  `threadId` snapshot shape used by scratch callers, and emits
  `AgentProgress::TaskBoardUpdated`.
- `schemas.rs` preserves the `openhuman.todos_*` JSON-RPC API.
- `tools.rs` preserves the granular `todo_*` agent tools.
- `runs.rs` owns the OpenHuman autonomous-run ledger, which is separate from
  task-board storage.

`agent::task_board` re-exports the TinyAgents board types and keeps the legacy
`TaskBoardStore` facade for existing callers. Legacy
`agent_task_boards/*.json` values are imported at startup through
`openhuman::agent::tinyagents::todos`; existing TinyAgents values are never replaced.
