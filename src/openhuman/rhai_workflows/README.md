# `rhai` — language-based workflows (Rhai `.ragsh` REPL)

Exposes TinyAgents' Rhai-backed `.ragsh` session runtime (the `repl` cargo
feature, `tinyagents::ReplSession`) as a first-class **`rhai_workflows` tool** so the
orchestrator model can author and execute its own workflow scripts — fan-out
over subagents, batched tool/model calls, loops, dedup/verify pipelines — and
run them deterministically, in the spirit of Claude Code Workflows and
Recursive Language Models.

One orchestrator tool call maps to **one `eval_cell`**: the model writes a Rhai
cell, the cell runs against a persistent per-session namespace (top-level `let`
bindings survive into the next cell), and the structured result flows back as
the tool result. The orchestrator's own turn loop *is* the CodeAct driver loop.

## Module shape

| File | Role |
| ---- | ---- |
| `mod.rs` | Exports only (no controller schemas in v1). |
| `types.rs` | `RhaiSessionId`, `RhaiEvalRequest`/`RhaiEvalResponse`, `RhaiLimitsOverride`, `RhaiCallSummary`, serde types. |
| `policy.rs` | Maps openhuman autonomy tier + `tool_timeout` clamps → `tinyagents::ReplPolicy` (fail-closed, bounded). |
| `bridge.rs` | Builds the `CapabilityRegistry<()>`: openhuman tools (approval-gated, scope-filtered) + provider models + subagents. |
| `sessions.rs` | `RhaiSessionManager`: LRU + idle-TTL bounded map of persistent `ReplSession`s, keyed `<thread>:<session_id>`, one cell at a time. |
| `ops.rs` | `eval_rhai_cell`: spawn_blocking + outer timeout, cancellation wiring, event forwarding, error → model-consumable result. |
| `tools.rs` | `RhaiTool` (the `rhai_workflows` tool: schema, permission, scope, timeout, display). |

## Fail-closed guarantees

Every failure mode returns a **model-consumable tool result** — never a panic,
never a hung turn:

- **Layered time bounds:** (1) rhai `on_progress` deadline for pure script
  loops; (2) `bridge_block_on` timer race for hung capability futures;
  (3) an outer `tokio::time::timeout(policy.timeout + 5s)` around
  `spawn_blocking`; (4) the harness `ToolTimeout::Secs` backstop above all of
  them. The tool's own timeout is always set below the harness backstop.
- **Bounded sessions:** LRU cap (16) + idle TTL (30 min); a second concurrent
  call on a busy session returns a typed "session busy" error rather than
  deadlocking; a poisoned/errored session is dropped, never reused.
- **Bounded work per session:** `ReplPolicy` caps on operations, output bytes,
  script bytes, and per-kind call counts. `full` tier may raise call-count
  limits up to a hard 2× ceiling via the tool's `limits` arg; `readonly` tier
  does not get the tool at all.
- **Cancellation end-to-end:** the turn's run-cancellation token drives a
  `ReplCancelFlag` watcher, so a user cancel drops an in-flight cell (script or
  capability call) promptly; the session is left resumable.

## Approval & security (bridged tools keep their own gates)

The Rhai bridge restricts callable tools to the parent turn's
`visible_tool_names`, and **excludes** recursion/duplication hazards: `rhai`
itself, `spawn_subagent`/`spawn_parallel_agents` (use `agent_query` instead),
and `run_workflow`/`await_workflow`. `ToolScope::CliRpcOnly` tools are denied.

Approval gating is **not** on the tinyagents repl bridge path (it lives in the
harness `wrap_tool` middleware, which the REPL bypasses), so the bridge itself
invokes `ApprovalGate::intercept_audited` (+ `record_execution`) for any tool
whose `external_effect_with_args` is true, failing closed on denial.

## Capability surface exposed to scripts

`model_query`, `tool_call`, `agent_query`, their `*_batched` variants, `emit`,
and `answer`. Graph authoring/execution (`graph_*`) is **out of scope for v1**
(the REPL's `graph_run` returns a reference, not an execution).

## Kill switch & rollout

The tool is **not registered** when the autonomy tier is `readonly` or when
`OPENHUMAN_RHAI_WORKFLOWS=0`; default-on for `supervised`/`full`.
`OPENHUMAN_RHAI=0` and `OPENHUMAN_RLM=0` are still honoured as legacy aliases.
Reverting the registration line disables the surface without touching the domain.
