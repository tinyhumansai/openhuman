# 07.1 — Sub-agent pipeline as a subgraph

Convert `subagent_runner/` "prepare prompt → filter tools → run child →
checkpoint/handback → mirror transcript" into an explicit graph.

Current status (2026-07-02): `tinyagents/subagent_graph.rs` defines and exports
the fixed pipeline topology, and `run_typed_mode` now runs that graph as a
best-effort diagnostic skeleton before continuing through the procedural runner.
The skeleton records the named phases with `GraphTracingSink`; the node effects
remain pending and can be moved over one phase at a time without changing the
external sub-agent behavior. The live child turn loop is still
`run_subagent_via_graph`: it already uses the shared TinyAgents harness for
steering, early exit, cap summaries, transcript persistence, and usage
aggregation, but it is not yet a `SubAgentSession`/`subagent_node` graph
implementation. The standalone `ops/usage.rs` glue file is deleted; the
remaining `AggregatedUsage` bridge lives with the graph route that produces it,
and the oversized-result `apply_handoff` helper now lives beside
`ResultHandoffCache` in `handoff.rs` instead of under `ops/`.

## Steps

1. Partially done: define `build_subagent_pipeline_graph` (now in
   `tinyagents/subagent_graph.rs` or under `subagent_runner/`): nodes
   `resolve_definition` (registry lookup + allowlist) → `prepare_context`
   (parent ctx, memory, action root, sandbox) → `assemble_prompt` →
   `expose_tools` (uses 01.3 selection middleware) → `run_child`
   (harness leaf via `subagent_node` or direct
   `run_turn_via_tinyagents_shared`) → `finalize` (checkpoint/
   awaiting-user handback, worker-thread mirror, handoff cache).
   Follow the established `build_*_graph` + `*_topology()` pattern; topology
   export exists, but node effects are still diagnostic placeholders.
2. Partially done: `run_typed_mode` (`ops/runner.rs`, the single chokepoint)
   invokes the compiled subgraph for diagnostic tracing before continuing
   through the procedural runner; `AgentGraph::Custom` runners still plug in as
   alternate `run_child` leaves.
3. Child lineage: run with parent depth/events (`invoke_in_parent`
   semantics) so `root_run_id` rollup + `SubAgentStarted/Completed/Reused`
   events are native; usage rollup via `ChildRun.usage` (feeds 06.3).
4. Fold `ops/{provider,prompt,checkpoint}.rs`
   plumbing into node implementations; `ops/graph.rs` shrinks to the leaf.
5. Reusable child sessions: map the follow-up/continue flows onto
   `SubAgentSession` (`send`, `transcript()`, `reset()`).

## Deletions

- Deleted: `subagent_runner/ops/usage.rs` glue.
- Deleted: `subagent_runner/ops/handoff_helper.rs` glue; `apply_handoff` moved
  to `subagent_runner/handoff.rs`.
- Remaining: parts of `ops/runner.rs`/`ops/graph.rs` absorbed by nodes (target:
  `ops/*` shrinks to policy nodes + tests).

## Acceptance

- Sub-agent e2e parity (ops_tests 1689-line suite green or rewritten
  against graph events); topology export validates; checkpoint/handback
  and worker-thread mirroring unchanged from the outside.
