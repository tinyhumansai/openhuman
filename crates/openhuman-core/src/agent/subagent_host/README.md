# Subagent host

OpenHuman adapters over the neutral TinyAgents sub-agent lifecycle. Given an
`AgentDefinition` and a task prompt, the runner reads the parent's task-local
`ParentExecutionContext`, resolves the child's model, filters the parent's
tool registry per the definition's `tools`, `disallowed_tools`, and
`skill_filter` (or inherits the parent's tools verbatim in fork mode), builds
a narrowed system prompt, runs the child turn on the TinyAgents harness, and
mirrors its transcript and progress back to the parent as one compact tool
result.

TinyAgents owns the generic sub-agent machinery: the lifecycle driver and
shared in-flight coordination, canonical pause and terminal persistence,
neutral role framing, generic tool restriction, result policy, and cap-hit
summary/digest behavior. OpenHuman adapts those contracts to product behavior.
It owns agent-definition and provider resolution, access-tier and dispatch
gates, tenant-scoped execution context, sandbox and workspace policy, artifact
authorization/materialization and path projection, billing and usage, session
and transcript mirroring, progress, and the public OpenHuman DTO/RPC contract.
The persistence adapter reads and migrates legacy OpenHuman checkpoints while
all new canonical pause and terminal writes go through TinyAgents.

## Layout

| File | Role |
| --- | --- |
| [`types.rs`](./types.rs) | Public product options, outcomes, errors, checkpoint data, modes, and usage |
| [`lifecycle.rs`](./lifecycle.rs) | Public host entry points, task identity/context wiring, and driver invocation |
| [`lifecycle/driver_adapter.rs`](./lifecycle/driver_adapter.rs) | OpenHuman planner/executor and neutral/product DTO conversion |
| [`lifecycle/driver_result.rs`](./lifecycle/driver_result.rs) | Product tool-result, status, cancellation, and durable-winner projection |
| [`lifecycle/persistence.rs`](./lifecycle/persistence.rs) | Legacy record reads/migration and product checkpoint metadata over TinyAgents persistence |
| [`ops/runner.rs`](./ops/runner.rs) | Product dispatch, typed/fork composition, provider/model policy, billing, and artifact host effects |
| [`ops/graph/dispatch.rs`](./ops/graph/dispatch.rs) | TinyAgents turn wiring, product middleware/context, transcript outcome and usage projection |
| [`ops/graph/transcript.rs`](./ops/graph/transcript.rs) | Tenant-scoped transcript/session bridge and worker mirror inputs |
| [`ops/checkpoint.rs`](./ops/checkpoint.rs) | Host model invocation and usage capture for TinyAgents cap summarization |
| [`ops/prompt.rs`](./ops/prompt.rs) | Product artifact-offload prompt contract appended to TinyAgents framing |
| [`ops/runner_artifacts.rs`](./ops/runner_artifacts.rs) | Tenant-authorized artifact storage, path projection, and result cap adapter |
| [`tool_prep.rs`](./tool_prep.rs) | Product prompt loading, deny/recovery precedence, delegation aliases, and host tool-order projection around TinyAgents restrictions |
| [`autonomous.rs`](./autonomous.rs) | OpenHuman autonomous iteration-cap policy |

TinyAgents also owns the character-safe `ResultPolicy` and the generic
cap-summary digest/fallback. OpenHuman retains artifact path guards,
tenant-backed storage, materialization under the parent's authorized action
root, user-visible path rendering, provider billing, and the legacy result
shape. Those product effects stay in the host adapter while the generic
framing, restriction, lifecycle, persistence, and summary mechanisms remain
in TinyAgents.

`SubagentRunOptions::run_context` carries the child's `OpenHumanRunContext`
explicitly; recursive execution passes this value rather than snapshotting
task-local scopes after a `tokio::spawn`, since task-local state does not
survive that boundary on its own.

## Where next

- `crate::agent_registry::host_definition` for `AgentDefinition` (and `tinyagents_definition` for `AgentTier`,
  `SandboxMode`), and the other archetype shapes this module resolves against.
- `crate::agent::orchestration` for `spawn_subagent` and the other
  user-facing tools that call into this runner.
- `crate::agent::session_host` for the parent turn that sets up the
  `ParentExecutionContext` a sub-agent reads.
- [`vendor/tinyagents`](../../../../../vendor/tinyagents/) (`tinyagents-orchestration::subagent`)
  for the lifecycle mechanics this
  module adapts rather than reimplements.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [The orchestrator](../../../../../docs/gitbooks/en/features/orchestration.md)
