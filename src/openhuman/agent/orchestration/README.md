# Agent Orchestration

`agent_orchestration` is the high-level control plane for agent-to-agent work.
It owns parent/child lineage, lifecycle state, wait/close/follow-up semantics,
and UI/diagnostic events. The lower-level `agent::harness` remains the execution
engine for prompt construction, policy-filtered tools, model selection, and
sub-agent loops.

## Current Inventory

- `agent_orchestration::tools::spawn_subagent` runs one typed sub-agent and returns a collapsed result.
- `agent_orchestration::tools::spawn_parallel_agents` fans out independent typed sub-agent runs.
- `agent_orchestration::tools::spawn_worker_thread` creates a persisted worker-thread transcript, but the current `spawn_subagent` tool rejects `dedicated_thread` until the worker UI is ready.
- `agent::harness::subagent_runner` is the canonical execution path for typed child agents.
- `agent::progress::AgentProgress::Subagent*` and `DomainEvent::Subagent*` already provide lifecycle and child tool-call telemetry.

## Control Surface

The intended canonical operations mirror Codex-style multi-agent controls:

- `spawn_agent`: register a child in the TinyAgents `DetachedTaskRegistry` and run it through `agent::harness::run_subagent`.
- `wait_agents`: wait for one or more children to reach a terminal state, optionally with a timeout. A terminal child is pruned from the registry by the wait that observed it.
- `abort_all`: publish `cancelled` on every live child and hard-abort its task.

The in-memory `list_agents` / `message_agent` / `close_agent` / `follow_up` /
`resume_agent` / `events` mirror was removed — the durable equivalents live in
`command_center::control`.

## State Model

Each child has a stable `orchestration_id`, an `agent_id`, optional
`parent_agent_id`, status, prompt, result summary, error, timestamps, and
metadata. Status is TinyAgents' `OrchestrationTaskStatus`; the terminal values
are `completed`, `failed`, `cancelled`, `timed_out`, and `abandoned`.

## Policy Inheritance

Policy inheritance is delegated to `agent::harness::run_subagent`, which already
derives child tools, model routing, sandbox context, spawn depth, and progress
from the parent `ParentExecutionContext`. The orchestration layer should only
add lineage and lifecycle semantics; it must not widen tool visibility beyond
what the harness exposes to the child.

## Persistence

The first implementation is process-local. The state shape is serializable so a
later PR can persist orchestration sessions across app restart, cron resumes, and
thread continuation without changing callers.
