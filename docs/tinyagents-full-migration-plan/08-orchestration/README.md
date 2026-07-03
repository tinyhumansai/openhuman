# 08 — Orchestration & graphs

Finish graph adoption: durable interrupts for approvals, workspace isolation
hooks, and the first per-agent `AgentGraph::Custom` runner. Parallel fanout,
`spawn_parallel_agents`, and graph-stub deletion already run on SDK graph
helpers / optional graph wiring.

Current status (2026-07-02): graph adapter internals such as the delegation
state machine and production delegation glue stay crate-internal; product
orchestration modules own the public RPC/tool surfaces and output shapes.

Target SDK surface: `graph::parallel::map_reduce` + `ParallelOptions`/
`FailurePolicy`/`ItemOutcome`, `Send`/`Command`/reducers/`ChannelSet`,
`Interrupt`/`ResumeTarget`/`Command::resume`, `WorkspaceIsolation`/
`WorkspaceDescriptor` + workspace events, `GraphTopology` export,
graph testkit.

Steps:

1. `01-map-reduce.md` — landed: `run_parallel_fanout` removed; callers use
   the SDK helper directly.
2. `02-spawn-parallel-graph.md` — landed: spawn_parallel_agents as validate→
   dispatch→worker→collect→finalize.
3. `03-interrupt-resume.md` — approval pauses as durable interrupts.
4. `04-graph-stubs.md` — default stubs deleted; land the first per-agent
   bespoke `AgentGraph::Custom` runner.
5. `05-worktree-isolation.md` — worktrees behind `WorkspaceIsolation`.

Done when: every long-running orchestration has named nodes, topology export,
cancellation checks, the boilerplate stubs stay gone, and at least one
production agent owns a bespoke custom graph; fanout/parallel code paths stay
SDK-owned with deterministic order + failure policy.

Keep (product): workflow_runs/agent_teams/command_center product state + RPC
shapes; delegate/orchestration tool output formats; worktree policy.
