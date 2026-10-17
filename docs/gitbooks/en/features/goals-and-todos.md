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

## Coding workflow

For a multi-step repository change, the master agent tracks an objective and
acceptance criteria, implements changes, runs checks, repairs failures, and
reviews the diff before completing the goal. It can delegate bounded tasks to
the **Coding Worker** (`delegate_coding`), which uses the coding model route,
a sandboxed tool belt, its own context, and a 200-iteration cap. Independent
workers should own disjoint files or isolated worktrees; the parent verifies
the integrated result. Workers cannot spawn recursively or complete the parent goal.

For example: “In this repository, implement pagination, add tests for empty
and final pages, and keep testing and fixing until the checks pass. Delegate
independent changes where useful.” Provide the repository path and the checks
that define success. Existing user agent overrides can change the shipped
configuration.

### Durable desktop work

For a multi-step coding request the orchestrator uses `autonomous_work` to
record an objective and explicit acceptance criteria. The desktop schedules
idle-thread continuations every ten seconds. A task has no overall time or
turn limit: work can span hours or days while the app runs, the computer is
awake, and its model and tools are available. Each individual turn still has
its normal timeout and iteration limits. Committed transcripts and explicit
checkpoints survive app restarts; interrupted commands must be reconciled
against the actual repository before they are retried.

Continuations preserve the model, thinking level and temperature selected by
user turns, including after an app restart. Thinking effort is a provider
request, not a minimum thinking duration: support and actual reasoning depend
on the selected model. The managed route preserves distinct effort values
instead of collapsing extra-high into high. Worker turns inherit the thread's
thinking choice. The web backstop is 65 minutes by default, above the harness's
60-minute turn limit; the model-call limit remains 15 minutes. These are
per-turn guards, not a lifetime limit on autonomous tasks.

Marked tool calls emitted as text are recovered through a literal-only parser
and still pass normal schema validation and permission checks. Known tools can
use JSON, parenthesized calls or named literal arguments; unknown tools,
expressions and malformed arguments are not executed. Protocol failures receive
bounded automatic repair with the actual tool catalog. Repeated incompatible
output becomes a visible blocker rather than a successful code-only answer.

### Component plans and production quality

Complex projects can be decomposed with `autonomous_work plan_task` into a
persistent hierarchy of up to ten levels and 2048 work packages. Each package
has its own acceptance criteria, parent, dependencies, state and evidence.
The host rejects unknown dependencies and cycles, including cycles created by
parent/child completion requirements. `task_start` requires verified
dependencies. `task_review` schedules a separate verification turn; successful
host-observed checks plus criterion evidence verify that component, not the
whole project. Parents cannot pass before their children. The final project
review is available only after all planned packages pass. `task_reopen`
invalidates affected ancestors and consumers so integration must be checked
again. Status supports `task_id` and `offset`; plan responses are paged to avoid
overloading smaller context windows.

The agent should expand complex work progressively: intent, measurable quality
targets, architecture, systems, components, implementation and tests. Persist
design and art-direction decisions, interface contracts, references and license
provenance, experiments and benchmark results in project Markdown. For visual
or interactive work, inspect actual rendered behavior; for simulation, test
numerical and boundary behavior. Research a current benchmark before claiming
state-of-the-art quality. Tool availability, longer runtime and deeper plans
cannot guarantee that any particular model can achieve that quality.

AGENTS.md instructions and SKILL.md/WORKFLOW.md bundles are supported through
the existing instruction and skill systems. Markdown notes, including notes
usable by Obsidian, are ordinary project files; this does not imply an Obsidian
plugin integration. Coding workers have a deliberately focused tool belt;
vision, image creation, browser/desktop checks and skill discovery are routed
through the parent and its available specialists. Missing credentials, tools,
model capabilities or failed evaluations remain explicit blockers.

The workflow is implementation → separate review → completion. The review
pass inspects the deliverable, reruns relevant checks and supplies evidence
for each criterion. The host additionally requires observed successful tool
activity, including a command, and rejects a review with failed tool calls.
`goal_complete` cannot bypass this gate. These checks improve accountability;
they are not a proof that code is correct or a guarantee of model quality.
Worker and external-agent summaries, including Antigravity, must be audited
against actual changes and results. Placeholders or demos do not satisfy a
request for production behavior.

The Stop button pauses the durable task as well as cancelling the active
turn. It stays paused after a restart and requires a user instruction to
resume. Deleting its thread also stops it. Authentication or permission
failures and repeated turns with no observable progress become explicit
blockers; temporary provider errors retry with backoff. A user-specified token
budget still stops work; no budget is invented automatically. Two ordinary
turn lanes can be occupied before background admission waits. No model runs
while the app is closed or the computer sleeps.

Ask “show autonomous work status” for the saved phase and checkpoint, or
“resume the autonomous task” after resolving a blocker. Status is also emitted
into the conversation after each continuation. The existing conversation UI
is used; there is no separate task dashboard.

## See also

- [Memory](memory.md): durable preferences and facts live there as learnings; goals and autonomous checkpoints are persisted per thread; todos describe the active session.
- [Native tools](native-tools/README.md): where the `todo` and `goal_*` tool calls sit among the rest.
- [Agent harness](../developing/architecture/agent-harness.md): the stores behind both.
