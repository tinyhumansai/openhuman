# 08.4 — Per-agent graphs: stubs out, bespoke in

Default-only `agent_registry/agents/*/graph.rs` stubs return
`AgentGraph::Default` (~13 lines each). Delete the boilerplate; keep the
seam; land at least one real custom graph before declaring the migration
complete so the cleanup does not remove an unproven seam.

Current status (2026-07-02): `BuiltinAgent.graph_fn` is optional and the
loader supplies `AgentGraph::Default` when it is absent. All 32
`agent_registry/agents/*/graph.rs` default stubs are gone, along with the five
default-only non-registry graph modules (`tinyplace_agent`, skill setup,
skill executor, agent memory, subconscious). `researcher` is now the first
bespoke production per-agent graph: it resolves to `AgentGraph::Custom`, runs a
compiled `route_research -> run_research_turn -> finalize` topology, exports
that topology through `agent.graph_topologies`, and delegates the actual
model/tool loop to the shared default sub-agent leaf so transcript persistence,
progress, handoff, cap-summary, and usage-rollup parity stay intact. Other
production orchestration graph topologies (delegation/workflow/team/
spawn-parallel) also exist.

## Steps

1. Done: land the first bespoke per-agent graph. `researcher/graph.rs` is an
   `AgentGraph::Custom` runner over a compiled graph with topology export.
   Remaining route sophistication: split search/read/synthesize/cite decisions
   into richer nodes once those policies move out of prompt text.
2. Done: make `graph_fn` optional on `BuiltinAgent` (default =
   `AgentGraph::Default` supplied by the loader/registry); delete every default
   stub `graph.rs` whose agent has no custom graph.
3. Done: `agent.graph_topologies` returns an `agents` array showing which graph
   (`default`/`custom`) each built-in resolves to.

## Deletions

- Deleted: all default-only `agent_registry/agents/*/graph.rs` files and the
  `graph_fn` boilerplate wiring for them.
- Deleted: default-only graph modules for `tinyplace_agent`, `skill_setup`,
  `skill_executor`, `agent_memory`, and `subconscious`.

## Acceptance

- Registry loads all agents with correct graph resolution (test);
  first bespoke graph is in production and exported; richer route tests +
  topology snapshot remain before declaring 08.4 complete.
