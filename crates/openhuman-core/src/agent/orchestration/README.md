# orchestration

`agent::orchestration` hosts live OpenHuman adapters for detached sub-agents:
spawning, cancellation, progress, and durable
subagent-session projections. TinyAgents owns generic task execution and
registry behavior; OpenHuman supplies product policy, execution context, RPC,
and UI-facing projections.

## Responsibilities

- Host adapters for detached spawns and durable session reuse.
- Background sub-agent task tracking, cancellation, completion
  delivery, and run-ledger projection.
- Manual cancellation for detached `spawn_subagent` runs.

## Key files

- `running_subagents.rs` and `running_subagents/`: admission checks,
  driver-backed cancellation, task ledger, and boot reconciliation.
- `background_completions.rs`, `completion_notice.rs`, and
  `background_delivery.rs`: host delivery policy over TinyAgents completion
  routing.
- `subagent_control.rs`: manual cancel RPC over the driver-owned job registry.
- `subagent_sessions/`: durable session reuse and deduplication.
- `tools.rs`: callable sub-agent tools.

## RPC

| Namespace | Surface |
| --- | --- |
| `subagent` | Cancel |

## Tools

The model-facing surface is the always-asynchronous `spawn_subagent` tool.
The parent reuses a `task_key` to continue a clarification or follow up on a
completed worker. Execution routes through TinyAgents' shared driver; OpenHuman
provides product policy, durable session resolution, progress, and cancellation.

## Persistence and policy

TinyAgents' session ledger stores `agent_runs` and `run_events`. OpenHuman
keeps scoped sub-agent session records and detached task metadata, and projects
terminal events into the product AgentRun view. Child tools, model routing,
sandbox context, spawn depth, and progress derive from the parent
`ParentExecutionContext`; this module must not widen harness tool visibility.

## Used by

- `agent/registration.rs` registers the callable tools; `tools::ops` composes that family.
- `threads` cancels detached work when a thread closes.
- `crate::core::all` registers the remaining controller pairs.
- The frontend uses the background run and sub-agent cancellation controls.
