---
description: >-
  Long-term goals, per-thread objectives with budgets, and a kanban task board -
  how OpenHuman stays pointed at what matters.
icon: target
---

# Goals & Todos

OpenHuman keeps the agent aligned with what you actually care about through three complementary layers: durable **long-term goals**, a single **thread goal** per conversation, and a collaborative **task board** of todos. Each one is editable by both you and the agent, and all of them survive restarts.

***

## Long-term goals

A short, human-readable list of your durable objectives, things like _"Ship the desktop app"_ or _"Grow the community to 10k."_ It lives as a plain Markdown file (`MEMORY_GOALS.md`) in your workspace, so you can open and edit it directly.

The list is deliberately tiny, capped at roughly **8 items / 500 tokens**, so it's cheap for the agent to read on every relevant turn and easy for you to keep honest. Each goal gets a stable short id (`g1`, `g2`, …) so it can be edited or deleted without depending on order.

* **Goals Panel** (Intelligence → Goals) shows the list with add / edit / delete actions.
* **Reflect** runs a background `goals_agent` that reviews your goals against recent memory and conversation, then makes minimal, justified changes: adding what you've clearly started pursuing, retiring what you've dropped. On first run it bootstraps an initial set from your context.
* The agent reads and updates the same list mid-conversation via its `goals_list` / `goals_add` / edit tools, so your edits and the agent's stay in lock-step.

RPC surface: `openhuman.memory_goals_list` / `_add` / `_edit` / `_delete` / `_reflect`.

***

## Thread goals

Each conversation can carry **one** thread goal: a durable "completion contract" the agent works across turns, interrupts, resumes, and budget boundaries. A thread goal has an objective, a status, and an optional **token budget** so you can cap how much work a thread is allowed to consume.

| Status            | Meaning                                                              |
| ----------------- | ------------------------------------------------------------------- |
| `active`          | The agent may keep working the objective.                           |
| `paused`          | Suspended (e.g. you interrupted); reactivates when the thread resumes. |
| `budget_limited`  | Tokens spent ≥ budget; substantive work halts until you raise it.   |
| `complete`        | Objective satisfied.                                                |

The orchestrator sets a goal with `goal_set`, reads it with `goal_get`, and finishes it with `goal_complete`. Updates emit `thread/goal/updated` events so the UI stays live.

**Autonomous idle continuation.** If a thread has an active goal and goes idle (no in-flight turn, no activity for a configured interval, e.g. 10 minutes), the [heartbeat](subconscious.md) can inject a single continuation turn that resumes the transcript and keeps working the objective. It's opt-in (`heartbeat.goal_continuation_enabled`) and guarded by a one-shot suppression flag per idle period, so the agent never self-drives into a loop.

***

## Task board (todos)

Every conversation also hosts a **kanban-style task board**: a list of discrete work cards that you and the agent build together. Unlike a thread goal (one durable objective), the board is a collection of concrete items, each with rich structure:

* Title / description, and a **status**: `todo`, `in_progress`, `awaiting_approval`, `ready`, `blocked`, `done`, `rejected`.
* Optional objective and desired outcome.
* An ordered **execution plan**, **acceptance criteria** checklist, assigned agent, and an **approval mode** (required / not required).
* Notes, blocker reason, and evidence / links.

The agent reads the board with `todo_list`, appends with `todo_add`, edits with `todo_edit`, and advances status with `todo_update_status`. Destructive operations (clear / remove / replace) are disabled by default. You and the agent share the same persistence, so edits stay consistent.

Two reserved boards back special views:

* **`user-tasks`**: your personal task list, not attached to any conversation. Create and manage these from the **User Task Composer** (Intelligence → Tasks). You can optionally attach a task to a conversation and assign it to the orchestrator with `approvalMode: not_required`, so the background dispatcher auto-picks and runs it.
* **`task-sources`**: an inbox for tasks ingested from external sources before they're promoted to an agent workstream.

RPC surface: `openhuman.todos_list` / `_add` / `_edit` / `_update_status` / `_set_session_thread`. Responses include a rendered `markdown` field so the board renders identically in the UI and in agent transcripts.

***

## How the three relate

| Layer              | Scope                | Count           | Who drives it                       |
| ------------------ | -------------------- | --------------- | ----------------------------------- |
| Long-term goals    | Your whole account   | ~8 max          | You + periodic `goals_agent` reflect |
| Thread goal        | One conversation     | 1 per thread    | Orchestrator, with optional budget  |
| Task board (todos) | One conversation     | Many cards      | You + agent, collaboratively        |

***

## See also

* [Subconscious Loop](subconscious.md): the background loop that powers idle continuation and task evaluation.
* [Memory Tree](obsidian-wiki/memory-tree.md): what goal reflection reads from.
* [SuperContext](super-context.md): first-turn grounding that complements goal-driven work.
