# 08.2 — `spawn_parallel_agents` as a graph tool

The spec's Phase 6 node shape is now live on SDK graph helpers; remaining work
is parity evidence and cancellation coverage.

Current status (2026-07-02): `spawn_parallel_agents` already fans prepared
workers through `tinyagents::graph::parallel::map_reduce`, exports the fixed
`validate -> dispatch -> worker -> collect -> finalize` topology, and the graph
entrypoint now owns `Config.action_dir` resolution for worktree-isolated workers.
Each result now carries explicit per-task lineage (`parentSession`,
`rootSession`, `childTaskId`) so graph/status consumers can connect child task
ids back to the root run. The graph now owns the live
`validate`/`dispatch`/`worker`/`collect`/`finalize`
phases: the graph entrypoint resolves parent context and registry state,
validate parses task requests and enforces the parent `max_parallel_tools`
limit, dispatch validates/preflights workers from an owned agent-definition
snapshot, and dispatch now rejects shared-workspace workers that can use
write/execute tools unless the target definition is `sandbox_mode = "read_only"`,
the task explicitly requests `isolation = "worktree"`, or the task provides
non-overlapping `files:` ownership for the shared-workspace serial fallback.
Worker fanout still uses the SDK `map_reduce` helper for parallel-safe batches,
shared write fallback runs the prepared batch in deterministic task order and
now checks the graph cancellation token between serial workers, collect still
projects compatibility `DomainEvent`/`AgentProgress`, and finalize still
returns the existing JSON shape. Parallel batches pass the same token into SDK
`ParallelOptions::with_cancellation`, so cancelled map-reduce runs surface as a
graph `Cancelled` outcome instead of an opaque fanout error. The tool wrapper
now inherits the live TinyAgents run cancellation token through OpenHuman's
scoped harness context and passes it into the graph entrypoint, falling back to a
local token only for direct/manual tool calls outside a run. The tool wrapper
still owns `ToolResult` translation so
malformed-argument and public error shapes stay unchanged. The unused pre-graph
public wrappers have been removed, internal graph helpers have been narrowed,
and the remaining shrink target is the 1280-line graph implementation.

## Nodes

- `validate`: parse tasks, enforce min/max count, and preserve malformed-args
  error ordering before parent-context lookup.
- `dispatch`: resolve per-task agent definitions, `subagents.allowlist`,
  toolkit requirements, ownership prompt boundaries, and worktree preflight.
- `worker`: the 07.1 subagent pipeline subgraph with inherited policy,
  optional `worktree_action_dir`, child task id, bounded budgets
  (`SubAgentBudget`).
- `collect` (reducer/`ChannelSet`): successes, failures, elapsed,
  iterations, worktree status, changed files, stale-read markers —
  deterministic task order.
- `finalize`: compatibility `DomainEvent`/`AgentProgress` projections,
  overlap warnings, existing `parallel_agents` JSON shape.

## Steps

1. Done: `build_spawn_parallel_graph` + topology export (established pattern).
2. Done: tool wrapper in `agent_orchestration/tools/spawn_parallel_agents.rs` is now
   a thin shell: schema → run graph → translate `ToolResult`.
3. Partially done: policy (spec "ownership and scheduling"): read-only workers
   may share workspace; worktree-isolated writers stay on `map_reduce`;
   shared-workspace writers with disjoint `files:` ownership are admitted but
   force a deterministic serial batch fallback; shared writers without
   parseable/disjoint ownership are rejected. Cancellation now has a graph-level
   token seam, named-node boundary checks, serial fallback checks between
   workers, SDK map-reduce cancellation wiring, and live parent run cancellation
   inheritance through the tool path. No widening of tools/model/sandbox/budget.
4. Checkpointing optional (fanout = Async durability).

## Deletions

- Orchestration mechanics inside `spawn_parallel_agents.rs` have moved into
  `agent_orchestration/spawn_parallel_graph.rs`; overlap detection lives in the
  collect/finalize graph path.

## Acceptance

- Required before completion: spawn_parallel suite green against identical JSON output;
  status consumers render per-task lineage.
