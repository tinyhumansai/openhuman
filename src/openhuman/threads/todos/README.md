# todos

Compatibility surface for OpenHuman task-board callers.

The task-board model and behavior are owned by
`tinyagents_graph::todos`: types, normalization, markdown rendering, CRUD,
plan decisions, session links, the single-`in_progress` invariant, atomic
claims, durable storage, and the in-memory scratch board.

OpenHuman keeps this module to preserve app-specific integration:

- `ops.rs` maps `BoardLocation` onto TinyAgents stores, preserves the optional
  `threadId` snapshot shape used by scratch callers, and emits
  `AgentProgress::TaskBoardUpdated`.
- `schemas.rs` preserves the `openhuman.todos_*` JSON-RPC API.
- `tools.rs` preserves the granular `todo_*` agent tools.
- `runs.rs` binds the TinyAgents autonomous-run ledger
  (`tinyagents_graph::todos::runs`) to `BoardLocation` addressing, renders its
  timestamps as RFC 3339 for the wire, publishes `TaskRunReclaimed`, and imports
  the retired `agent_task_boards/<hex>.runs.json` ledgers. The run record,
  heartbeat, staleness policy, and reclaim sweep are the crate's.

`agent::task_board` re-exports the TinyAgents board types and keeps the legacy
`TaskBoardStore` facade for existing callers. Legacy
`agent_task_boards/*.json` boards are imported at startup through
`openhuman::agent::tinyagents::todos`, and the `*.runs.json` ledgers beside them
through `threads::todos::runs::migrate_legacy_task_runs`; existing TinyAgents
values are never replaced.
