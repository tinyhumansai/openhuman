# 07 — Sub-agents, steering, recursion

Re-express the sub-agent pipeline on crate primitives; collapse the
detached-run registry onto durable task stores; one recursion authority.

Target SDK surface: `SubAgent::invoke_in_parent` (lineage/depth/event
inheritance), `SubAgentSession` (multi-turn reuse), `SubAgentTool` (depth
guard), `graph::subagent_node` + `SubAgentPolicy`/`SubAgentBudget`,
`graph::subgraph` (`shared_subgraph_node`/`adapter_subgraph_node`),
`TaskStore`/`JsonlTaskStore` + 10 `orchestrate_*` tools +
`SteeringRegistry`, `SteeringCommand`/`SteeringHandle`/`SteeringPolicy`,
`RecursionPolicy`/`RecursionStack`, `CancellationToken`.

Steps:

1. `01-subagent-pipeline.md` — build pipeline as a subgraph.
2. `02-detached-taskstore.md` — running_subagents onto durable TaskStore.
3. `03-steering-recursion.md` — steering tools + one recursion cap.

Keep (product): agent definition lookup, allowlist enforcement, archetype
prompt assembly, memory context, sandbox/action-root narrowing,
worker-thread mirroring, integrations preflight, handoff cache. These become
named nodes/adapters, not deleted.

NOTE (memory, P5 2026-06-30): wrapping crate `SubAgentTool` around a bare
`Arc<AgentHarness>` was DECLINED because it discarded the pipeline. The
subgraph approach keeps the pipeline as explicit nodes — that's the
difference that makes it viable now.
