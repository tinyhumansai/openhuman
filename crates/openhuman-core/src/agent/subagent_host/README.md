# Subagent host

OpenHuman adapters over the neutral TinyAgents sub-agent lifecycle. Given an
`AgentDefinition` and a task prompt, the runner reads the parent's task-local
`ParentExecutionContext`, resolves the child's model, filters the parent's
tool registry per the definition's `tools`, `disallowed_tools`, and
`skill_filter` (or inherits the parent's tools verbatim in fork mode), builds
a narrowed system prompt, runs the child turn on the TinyAgents harness, and
mirrors its transcript and progress back to the parent as one compact tool
result.

This module owns product policy and effects: definition resolution, prompt
assembly, tool narrowing, model selection, workspace and sandbox setup,
artifacts, checkpoints, and progress. `tinyagents_orchestration::subagent`
owns lifecycle ordering, task-key coalescing, and mutually exclusive pause or
terminal persistence; this crate does not duplicate that state machine.

## Layout

| File | Role |
| --- | --- |
| [`types.rs`](./types.rs) | `SubagentRunOptions`, `SubagentRunOutcome`, `SubagentRunError`, `SubagentMode`, `SubagentCheckpointData`, `SubagentUsage` |
| `ops/` | `run_subagent`, the typed and fork execution modes, and the TinyAgents graph route |
| [`lifecycle.rs`](./lifecycle.rs) | `run_subagent`, `run_subagent_with_parent`, `continue_subagent`, checkpoint load/save, `OpenHumanSubagentHost` |
| [`tool_prep.rs`](./tool_prep.rs) | Tool filtering, prompt loading, and the prompt protocol block |
| [`autonomous.rs`](./autonomous.rs) | Iteration-cap policy for autonomous (non-interactive) sub-agent runs |

`SubagentRunOptions::run_context` carries the child's `OpenHumanRunContext`
explicitly; recursive execution passes this value rather than snapshotting
task-local scopes after a `tokio::spawn`, since task-local state does not
survive that boundary on its own.

## Where next

- `crate::agent::harness::host_definition` for `AgentDefinition` (and `tinyagents_definition` for `AgentTier`,
  `SandboxMode`), and the other archetype shapes this module resolves against.
- `crate::agent::orchestration` for `spawn_subagent` and the other
  user-facing tools that call into this runner.
- `crate::agent::session_host` for the parent turn that sets up the
  `ParentExecutionContext` a sub-agent reads.
- [`vendor/tinyagents`](../../../../../vendor/tinyagents/) (`tinyagents-orchestration::subagent`,
  `tinyagents-graph`) for the lifecycle and delegation-graph mechanics this
  module adapts rather than reimplements.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [The orchestrator](../../../../../docs/gitbooks/en/features/orchestration.md)
