# TinyAgents SDK Gaps

This document lists TinyAgents SDK features that are missing or only partially
available from the perspective of migrating OpenHuman's Rust agent core onto
TinyAgents.

Scope:

- Original source baseline: local TinyAgents checkout at `6f898fb`.
- Refresh note: TinyAgents 1.3.0 was re-verified from the published crate
  source. Several older "missing" items are now shipped in 1.2.0-1.3.0:
  tool policy metadata, recoverable unknown-tool calls, reasoning/tool-call
  stream deltas, orchestration task stores, budget reservation/reconciliation,
  ordered parallel map/reduce, sub-agent steering/task controls, workspace
  isolation hooks, and middleware control outcomes. The backlog below keeps only
  the residual OpenHuman migration pressure and SDK surface gaps.
- OpenHuman evidence: `src/openhuman/tinyagents/*`,
  `src/openhuman/agent/*`, `src/openhuman/cost/*`, and
  `src/openhuman/tokenjuice/*`.
- This is not the OpenHuman migration plan. That plan lives in
  `docs/tinyagents-migration-spec.md`.
- Items here are upstream TinyAgents implementation candidates.
- Tests should follow each migrated surface; only broad end-to-end parity suites
  should wait for the final cutover.

## Executive Summary

TinyAgents already has strong primitives for harness runs, graph execution,
middleware, event streams, model profiles, usage/cost accounting, checkpointers,
policy metadata, recoverable tool-call behavior, workspace isolation, and
sub-agent orchestration. The biggest remaining gaps are now narrower:
OpenHuman adapter migration, transcript/session migration, richer replay and
redaction rules, model/provider catalog integration, registry diagnostics, and a
few still-missing SDK fields.

OpenHuman can migrate more of `src/openhuman/agent/` by adopting the newer
TinyAgents surfaces and filling the remaining gaps:

- A free-form metadata map on `ToolSchema` for model-visible schema
  annotations; SDK-owned enforcement metadata now lives in `ToolPolicy`.
- A reasoning field on the middleware-facing `harness::model::ModelDelta`;
  `AgentEvent::ModelDelta` already carries nested `MessageDelta.reasoning`.
- A one-time migration path from old OpenHuman `session_raw/*.jsonl` and
  Markdown transcripts into TinyAgents store/journal/status records.
- Production replay rules over TinyAgents stores/status: redaction, cursors,
  backfill, cancellation, and OpenHuman controller compatibility.
- Storage compatibility options for SQLite users that already own a connection,
  schema, or native sqlite patch policy.
- A `root_run_id` field on harness `RunConfig`; lineage exists in graph and
  observability records, but not on the harness config itself.
- A money/USD field on `Usage`; token usage and cost totals remain separate.
- Provider/model catalog metadata that can drive preflight, fallback, and
  reconciliation.
- Conformance suites for providers, tools, middleware, graph stores, and
  checkpointers.

## Backlog

### 1. Rich Tool Policy Metadata

Status: shipped in 1.3.0; residual schema metadata gap remains.

TinyAgents now has SDK-owned `ToolPolicy`, `ToolSideEffects`, `ToolRuntime`,
`ToolAccess`, `WorkspaceAccess`, `SandboxMode`, `Tool::policy`, registry policy
snapshots, and `ToolPolicyMiddleware`. Strict policy can fail closed on
unclassified tools, enforce sandbox and result-byte requirements, and keep plain
`ToolSchema` as the model-visible projection.

Residual:

- `ToolSchema` still has no free-form metadata map for model-visible schema
  annotations or app-specific hints. Use `ToolPolicy` for enforcement metadata
  and keep this as the remaining SDK shape gap.
- OpenHuman still needs to map domain tool registry metadata into
  `Tool::policy` snapshots before deleting adapter-local policy plumbing.

### 2. Recoverable Unknown Tool Calls

Status: shipped in 1.3.0.

TinyAgents now has `UnknownToolPolicy::{Fail, ReturnToolError, Rewrite}` and
emits `AgentEvent::UnknownToolCall` with the original requested name and
arguments. This distinguishes "tool not found" from "tool executed and failed"
and lets a run keep going so the model can correct itself.

OpenHuman follow-up:

- Replace the adapter-local `__openhuman_unknown_tool__` sentinel with
  `UnknownToolPolicy` once the surrounding compatibility surface is migrated.

### 3. Reasoning And Tool-Argument Streaming

Status: shipped in 1.3.0; residual middleware event-shape gap remains.

TinyAgents `MessageDelta` now carries provider-neutral `text`, `reasoning`, and
`tool_call` channels, and `AgentEvent::ModelDelta` carries the delta with an
explicit `run_id` and `call_id`.

Residual:

- The middleware-facing `harness::model::ModelDelta` still has no `reasoning`
  field, and the agent loop drops reasoning when converting stream items into
  that middleware shape. Event consumers can read reasoning from
  `AgentEvent::ModelDelta.delta.reasoning`.
- OpenHuman still needs to route provider reasoning/tool-argument deltas through
  the TinyAgents stream before deleting UI-specific forwarding shims.

### 4. Durable Orchestration Task Store

Status: shipped as SDK primitives in 1.3.0; OpenHuman migration remains.

TinyAgents now has `TaskStore`, `InMemoryTaskStore`, `JsonlTaskStore`,
`OrchestrationTaskRecord`, lifecycle transitions, cancel/kill outcomes,
filters, graph/harness status with lineage, `graph::subagent_node`, and
`graph::subgraph`.

OpenHuman follow-up:

- Map durable sub-agent session rows and worker-thread records into TinyAgents
  task/status/journal records while preserving controller compatibility.
- Retire bespoke task status/tombstone persistence in `running_subagents.rs`
  only after restart/replay behavior is projected through the SDK records.

### 5. SQLite Storage Compatibility

Status: partially present.

TinyAgents 1.3 has `SqliteCheckpointer`, `from_connection`, and `schema_sql`,
and OpenHuman now enables the `sqlite` feature by aligning both Cargo worlds on
`rusqlite 0.40` / `libsqlite3-sys 0.38`. The remaining gap is not feature
enablement or basic schema access; it is ownership. OpenHuman still patches the
sqlite crates locally for the current toolchain, owns existing session/checkpoint
tables through `SqlRunLedgerCheckpointer`, and needs a clean way to adopt or
bridge SDK checkpoint storage without surrendering dependency or schema control.

Implement one or more OpenHuman compatibility paths:

- Provide a version-flexible storage layer, possibly via `sqlx` or a separate
  crate feature matrix.
- Expose a small `CheckpointStore` persistence trait below `Checkpointer`.
- Add an adapter/cutover path that can project OpenHuman run-ledger checkpoints
  into SDK checkpoint storage without breaking existing resume semantics.

Acceptance criteria:

- Applications that already own SQLite can use TinyAgents durable checkpoints
  without native-link conflicts.
- OpenHuman can replace `SqlRunLedgerCheckpointer` with an SDK-supported adapter
  or a thin schema integration.
- Storage features remain opt-in and keep the default crate dependency-light.

### 6. Production Event And Status Journals

Status: partially present.

TinyAgents has `HarnessEventJournal`, `StoreEventJournal`, `HarnessStatusStore`,
and `HarnessRunStatus`. OpenHuman still bridges TinyAgents events into its own
progress system, cost tracker, run ledger, and UI status stream.

Implement:

- Durable event journals with cursors, replay windows, filters, compaction, and
  redaction hooks.
- Status stores with parent/root lineage, thread-scoped listing, phase details,
  active tool/model call ids, usage totals, cost totals, and terminal summaries.
- Event filters for UI surfaces: text stream only, tool timeline, cost updates,
  graph lifecycle, errors, task lifecycle.
- Redaction policies for prompts, tool args, tool results, PII, secrets, and
  provider payloads.
- Stable event ids and offset semantics across process restarts.

Acceptance criteria:

- A UI can attach late and reconstruct a run without subscribing at start time.
- A supervisor can query every active descendant of a root run.
- OpenHuman event bridges become mostly format adapters, not state owners.

### 7. Cost, Usage, And Budget Enforcement

Status: shipped in 1.3.0; residual money field gap remains.

TinyAgents now has `Usage`, `UsageTotals`, `CostTotals`,
`BudgetLimits.max_cached_input_tokens`, budget middleware, and
`AgentEvent::{BudgetReserved, BudgetReconciled, BudgetWarning, BudgetExceeded}`.
The SDK can preflight, reserve, enforce, and reconcile token budgets.

Residual:

- `Usage` still has no USD/money field. Token usage and money remain separate
  (`Usage`/`UsageTotals` vs. `CostTotals`), so OpenHuman cost UI still needs a
  projection that joins token usage with pricing/cost records.

### 8. Model Catalog And Provider Resolution

Status: partially present.

TinyAgents has `ModelProfile`, including provider, model, modalities, tool
calling, streaming, structured output, reasoning, and token windows. OpenHuman
still has provider catalog logic and local model capability inference that drive
fallback, token budgeting, and routing.

Implement:

- SDK-owned model catalog snapshots with provider, model id, display name,
  lifecycle status, context windows, modalities, streaming support, reasoning,
  structured-output support, and pricing keys.
- Capability-driven model resolution: required capabilities, fallback chains,
  local/cloud preferences, and provider health.
- Runtime profile discovery hooks for local models.
- Pricing table integration that maps `ModelProfile` to `CostTotals`.

Acceptance criteria:

- Model selection can be expressed in TinyAgents policy instead of
  OpenHuman-only routing code.
- Fallback can reject models that lack required tool, vision, structured-output,
  context-window, or reasoning capabilities.
- Token budgeting can use the resolved model's real context window.

### 9. Dynamic Tool Exposure And Allowlist Policy

Status: partially present.

TinyAgents can run with a provided tool registry, but OpenHuman needs per-agent,
per-tier, per-sub-agent, and per-task allowlists. Tool visibility depends on
security tier, workspace roots, parent/child delegation policy, model
capabilities, and whether the run is background or interactive.

Implement:

- A tool selection middleware that receives run context, agent identity, task
  kind, parent policy, and model profile.
- Allowlist/denylist composition with explicit inheritance rules.
- Explainable exposure decisions for audit/debugging.
- Fail-closed behavior when policy metadata is missing.

Acceptance criteria:

- Sub-agents inherit only the tools they are allowed to call.
- Tool exposure decisions are visible in run events or observations.
- OpenHuman can remove adapter-local allowlist enforcement from most call paths.

### 10. Graph Fanout And Parallel Agent Ergonomics

Status: map/reduce helper shipped in 1.2.1-1.3.0; OpenHuman builder migration
remains.

TinyAgents now has ordered `map_reduce`, `FailurePolicy`, `ParallelOptions`,
max concurrency, per-item timeout, total timeout, and cooperative cancellation.
OpenHuman council runs and `spawn_parallel_agents` can use this helper directly.

Residual:

- The higher-level parallel-agent builder remains OpenHuman-specific policy
  glue: task validation, `Send` dispatch shape, result envelopes, usage/cost
  merging, and ownership/worktree policy adapters.

### 11. Sub-Agent Steering, Waiting, And Reuse

Status: SDK primitives shipped in 1.3.0; OpenHuman lifecycle projection remains.

TinyAgents now has sub-agent sessions/tools, steering, task stores, cancel/kill
control outcomes, graph/harness lineage, and reusable child-run primitives.

OpenHuman follow-up:

- Project existing detached run tracking, wait handles, user-facing
  cancellation, early-exit handling, and parent-child progress aggregation onto
  TinyAgents task/status records before reducing `running_subagents.rs`.

### 12. Workspace Isolation And Sandbox Hooks

Status: shipped in 1.3.0; OpenHuman policy integration remains.

TinyAgents now has `WorkspaceDescriptor`, `WorkspaceIsolation`,
`SharedRootWorkspace`, sandbox descriptors, `ToolExecutionContext.workspace`,
and `WorkspaceDescriptor::enforce(path, events)` which emits
`AgentEvent::WorkspaceViolation` and fails closed when a path leaves the allowed
roots.

OpenHuman follow-up:

- Implement OpenHuman's action-root, trusted-root, internal-workspace, worktree,
  sandbox, and command-tier policy as a `WorkspaceIsolation` provider and tool
  middleware projection.

### 13. Middleware Control Outcomes

Status: shipped in 1.3.0.

TinyAgents now has `MiddlewareControl::{StopWithFinal, Interrupt}`,
`RunContext::request_control`, precedence handling via
`MiddlewareControl::precedence()`, stable `kind()` labels, and
`AgentEvent::ControlApplied` so control decisions are visible in journals.

OpenHuman follow-up:

- Route early-exit tools and budget stop hooks through `MiddlewareControl`
  before deleting adapter-local steering side channels.

### 15. Registry Diagnostics And Introspection

Status: partially present.

TinyAgents has registry primitives. OpenHuman still needs richer diagnostics for
duplicate components, alias resolution, component health, model/provider/tool
capabilities, and event listener wiring.

Implement:

- Registry snapshot export with models, tools, middleware, graph nodes,
  checkpointers, task stores, event listeners, and aliases.
- Duplicate and shadowing diagnostics.
- Health/status probes for registered providers and stores.
- Machine-readable component dependency graph.
- Optional DOT/JSON graph export for runtime components, not only graph nodes.

Acceptance criteria:

- A CLI or UI can show exactly what TinyAgents components are active.
- Registry failures are actionable without inspecting app-specific logs.
- OpenHuman dead-code audits can map old modules to SDK-owned registry entries.

### 17. Storage And Graph Conformance

Status: partially present.

TinyAgents 1.3 includes storage conformance coverage for built-in checkpointers
and task stores, including SQLite under the feature. Durable OpenHuman adapters
and fuller graph behavior are still hard to migrate safely without shared
contract coverage.

Implement:

- Checkpointer conformance for OpenHuman adapters and caller-supplied stores.
- TaskStore conformance for lifecycle transitions, filters, cancellation,
  timeout, kill, restart/replay, and concurrent writes.
- Graph conformance for `Send`, reducers, interrupts, resume, max concurrency,
  dynamic routing, fanout failure policy, and deterministic result collection.

Acceptance criteria:

- Storage adapters can be swapped without changing graph behavior.
- Durable interrupt/resume semantics are proven across backends.
- Parallel-agent helpers have regression tests for order, failure, timeout, and
  cancellation.

## Implementation Order

1. Define API contracts for tool policy, unknown-tool handling, streaming delta
   channels, durable task storage, storage adapters, and control outcomes.
2. Implement the lowest-level data types and traits behind non-breaking
   defaults.
3. Add in-memory implementations first.
4. Add durable stores and compatibility adapters second.
5. Add middleware helpers and high-level graph helpers.
6. Migrate OpenHuman adapters to the new SDK surfaces.
7. Remove OpenHuman-specific compatibility shims once the SDK behavior is
   equivalent.
8. Implement conformance and regression tests last.
