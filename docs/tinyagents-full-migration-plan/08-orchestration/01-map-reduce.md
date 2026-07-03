# 08.1 — `run_parallel_fanout` → `graph::parallel::map_reduce`

The crate now has the ordered/bounded map-reduce helper the OpenHuman
helper was written to fill (`ParallelOptions`, `FailurePolicy`,
per-item `ItemOutcome`, cancellation).

Current status (2026-07-02): landed. Live fanout callers now invoke
`tinyagents::graph::parallel::map_reduce` directly: `spawn_parallel_agents`,
workflow phase fanout (`workflow_runs/engine.rs`), and model_council fanout.
`tinyagents/orchestration.rs::run_parallel_fanout` has been removed. The
remaining TaskStore re-export seam is crate-internal for detached-subagent
lifecycle bookkeeping. Workflow phase fanout now shares the durable workflow
stop signal with `ParallelOptions::with_cancellation`, so SDK cancellation maps
back to the existing `Interrupted` run state. Usage-rollup parity has direct
coverage in `turn_subagent_usage::map_reduce_fanout_preserves_scope`: TinyAgents
1.3.0 `map_reduce` drives bounded futures through `buffer_unordered` and
reorders in the same async call path, so OpenHuman's tokio task-local
parent-turn collector stays visible to inline `spawn_parallel_agents` workers.

## Steps

1. Done: compare `tinyagents/orchestration.rs::run_parallel_fanout` semantics
   (input-order results, bounded concurrency, take-once payload cell,
   graph events) with `map_reduce`.
2. Done: re-point callers: `spawn_parallel_agents`, workflow phase intra-phase
   fanout (`workflow_runs/engine.rs`), model_council fanout.
3. Done: preserve the task-local usage-collector behavior. The direct
   `turn_subagent_usage::map_reduce_fanout_preserves_scope` regression proves
   records made inside `map_reduce` workers still roll into the active parent
   turn collector. The longer-term 06-cost work can replace this task-local
   bridge with `UsageTotals`/`ChildRun` lineage once lineage parity lands.
4. Done: choose `FailurePolicy` per caller (council = collect-all; workflow
   phase = fail-fast or per-phase config). Use the 1.3.0 options:
   `with_item_timeout`/`with_total_timeout`/`with_cancellation`
   (a per-item timeout is behavior the old helper never had — set it
   deliberately per caller, not by default). Workflow phase fanout now uses
   `with_cancellation`; timeout policy remains unset by design.

## Deletions

- `run_parallel_fanout` + the move-once cell machinery in
  `tinyagents/orchestration.rs` (file keeps the TaskStore re-exports until
  07.2 relocates them).

## Acceptance

- Deterministic input-order results with out-of-order completion; task-local
  usage rollup parity on a fanout turn.
