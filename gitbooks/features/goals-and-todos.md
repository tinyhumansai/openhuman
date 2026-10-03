---
description: The agent's session todo list and per-thread goals.
icon: target
---

# Goals & Todos

## Agent work state

While it works on a multi-step request the agent keeps a session todo list,
the same shape Claude Code and Codex use: one `todo` tool call writes the whole
list (`content` + `pending` / `in_progress` / `completed`), scoped to the agent
session and held in memory for the life of the process. Thread goals are the
per-thread completion contract (`goal_set` / `goal_get` / `goal_complete`).

Neither is a kanban board. There is no per-thread task board, no card CRUD,
no approval gate, and no `thread_goals`, `todos`, or `threads_task_board` RPC
endpoint. Conversation threads remain the chat/session container.

## In the chat pane

Both show above the composer while the agent works, read-only — the agent
owns them, the pane reflects them:

- The **todo checklist** lists every step with its state: completed items
  strike through and stay, the one `in_progress` item is marked, and the
  header counts how many are done. It collapses to that header.
- The **goal banner** shows the objective, its status (active, paused, budget
  reached, complete) and tokens used against the budget when one was set.

Neither has an RPC of its own. Each tool call answers with its state as JSON,
so the pane reads the newest `todo` / `goal_*` tool result in the thread —
across the live turn and the thread's persisted turns, which is what keeps a
goal on screen for the many turns after the one that set it.

## See also

- [Memory](memory.md): durable preferences and facts live there as learnings; goals and todos are in-session only.
