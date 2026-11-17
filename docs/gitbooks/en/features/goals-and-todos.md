---
description: >-
  The agent's session to-do list and per-thread goals: work state the chat
  pane shows you, not a task board you manage.
icon: target
---

# Goals and to-dos

The agent tracks its own work in two places. A session to-do list holds the steps of the request it is handling. A per-thread goal holds the objective that thread is trying to finish.

## How the agent uses them

While it works on a multi-step request, the agent keeps a session to-do list in the same shape that Claude Code and Codex use. One `todo` tool call writes the whole list. Each item has `content` and a state: `pending`, `in_progress` or `completed`. The list belongs to the agent session and stays in memory while the process runs.

Thread goals are the completion contract for a thread. The agent sets, reads and completes them with `goal_set`, `goal_get` and `goal_complete`.

Neither is a kanban board. There is no per-thread task board, no cards to edit, and no `thread_goals`, `todos` or `threads_task_board` RPC endpoint. Conversation threads are still the chat container.

## In the chat pane

Both show above the composer while the agent works. They are read-only. The agent owns them and the pane reflects them.

- The to-do checklist lists every step with its state. Completed items are struck through and stay in the list. The `in_progress` item is marked, and the header counts how many are done. You can collapse it to the header.
- The goal banner shows the objective, its status (active, paused, budget reached or complete), and tokens used against the budget if one was set.

Neither has an RPC of its own. Each tool call returns its state as JSON, and the pane reads the newest `todo` or `goal_*` result in the thread, across the live turn and earlier turns. That is why a goal stays on screen for many turns after the one that set it.

## See also

- [Memory](memory.md): durable preferences and facts live there as learnings. Goals and to-dos last only for the session.
- [Native tools](native-tools/README.md): where the `todo` and `goal_*` tool calls sit among the rest.
- [Agent harness](../developing/architecture/agent-harness.md): the stores behind both.
