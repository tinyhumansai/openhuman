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

- **Layered time bounds, strictly ordered `inner < outer < harness`:**
  (1) the **inner** deadline — rhai `on_progress` for pure script loops and
  the `bridge_block_on` timer race for hung capability futures, both driven
  by `policy.timeout` (`policy.rs::resolve_policy`); (2) the **outer**
  `tokio::time::timeout` around `spawn_blocking` in `ops.rs`, at
  `policy.timeout + OUTER_TIMEOUT_GRACE_SECS` (`policy::outer_backstop_secs`);
  (3) the **harness** `ToolTimeout::Secs` backstop the agent tool-execution
  loop enforces, at `outer_backstop_secs + HARNESS_TIMEOUT_GRACE_SECS`
  (`policy::harness_backstop_secs`, wired in `tools.rs::timeout_policy`) —
  strictly above both inner layers. Each grace exists so the *next* layer out
  only ever fires if every layer inside it already failed to enforce its own
  deadline: the inner deadline is the primary enforcement point, the outer
  backstop defends against a hung/poisoned inner layer (dropping the session
  on fire), and the harness backstop is a last resort — if it ever won the
  race it would drop the whole tool-execution future, skipping the
  `RhaiError::Timeout` taxonomy, `finish_cell` accounting, `close_session`,
  and the outer-backstop session cleanup entirely (E-M1).
- **Bounded sessions:** LRU cap (16) + idle TTL (30 min); a second concurrent
  call on a busy session returns a typed "session busy" error rather than
  deadlocking; a poisoned/errored session is dropped, never reused. Neither
  sweep ever evicts a slot whose cell is currently in flight (its session
  `Mutex` held) — the cap/TTL can be exceeded, logged, while every live slot
  is genuinely busy, but a running cell's session is never pulled out from
  under it (E-m6).
- **Bounded work per session:** `ReplPolicy` caps on operations, output bytes,
  script bytes, and per-kind call counts. `full` tier may raise call-count
  limits up to a hard 2× ceiling via the tool's `limits` arg; `readonly` tier
  does not get the tool at all.
- **Reused sessions keep their own policy:** `ReplSession` exposes no
  re-policy operation after construction (only builder-style `with_policy`),
  so `sessions::SlotHandle` carries the policy a session was actually built
  with. A cell that reuses a `session_id` with a *different* resolved policy
  (e.g. a different `timeout_secs`/`limits`) logs a warning and every bound
  for that call — the outer backstop, `limits_remaining` — is computed from
  the session's live policy, never the newly-resolved one (E-M2). This is
  deliberate: keying the session cache on a policy fingerprint instead would
  silently fragment sessions and lose bindings by design.
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
