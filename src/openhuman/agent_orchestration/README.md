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

- `spawn_agent`: create a child record and run it through `agent::harness::run_subagent`.
- `list_agents`: return child snapshots ordered by creation time.
- `message_agent`: record parent-to-child communication for the orchestration record. Live mid-turn input is not yet injected into the running harness loop.
- `wait_agents`: wait for one or more children to reach a terminal state, optionally with a timeout.
- `close_agent`: close the orchestration record and abort the background run when one exists.
- `resume_agent`: spawn a linked continuation child from an existing child record.
- `follow_up`: spawn a linked child using the prior child result as context unless explicit context is supplied.

## State Model

Each child has a stable `orchestration_id`, an `agent_id`, optional
`parent_agent_id`, status, prompt, message log, result summary, error, timestamps,
and metadata. Terminal statuses are `completed`, `failed`, `cancelled`, and
`closed`.

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
