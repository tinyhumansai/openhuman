# 06 — Usage, cost, budgets

Finish moving accounting onto crate primitives; OpenHuman keeps the tracker
DB, dashboard RPC, and pricing policy.

Target SDK surface: `Usage { input_tokens, output_tokens, total_tokens,
cache_read_tokens, cache_creation_tokens, reasoning_tokens }`, `UsageTotals`,
`CostTotals`, `BudgetMiddleware`/`BudgetLimits`/`BudgetTracker`,
`AgentEvent::{UsageRecorded, CostRecorded, BudgetReserved, BudgetReconciled,
BudgetWarning, BudgetExceeded}`,
`ChildRun.usage`/`RunTree` lineage rollup, `ModelPricing` (catalog, incl.
cache/reasoning rates).

The $0-cost bug and the unobserved-turn tracker gap are already fixed.
Remaining:

Current status (2026-07-02): tinyagents 1.3.0 exposes the target budget and
usage primitives, but OpenHuman has not wired `BudgetMiddleware`,
`BudgetLimits`, `ChildRun`, or `RunTree` into the shared runner. TinyAgents
runtime already emits `AgentEvent::UsageRecorded` after each model call, so the
OpenHuman gap is ownership/de-duplication of that event stream, not a separate
usage-accounting middleware install. Keep the current OpenHuman cost seams live
for now:
crate-internal `CostBudgetMiddleware` gates already-exceeded daily/monthly
budgets on every shared turn, `OpenhumanEventBridge::record_usage` records
`UsageRecorded` into the global tracker, and crate-internal
`turn_subagent_usage` folds child spend into the parent turn footer and
in-memory `LastTurnUsage` web footer payload. Transcript/session metadata
persistence is separate. The bridge now logs TinyAgents budget/cost events
(`BudgetReserved`, `BudgetReconciled`, `BudgetWarning`, `BudgetExceeded`,
`CostRecorded`) without feeding them into OpenHuman accounting. Installing the
crate budget middleware before event de-duplication would risk double-counting
`UsageRecorded`.

Local inventory: there is no local `src/openhuman/tinyagents/cost*` adapter
module; TinyAgents itself has `tinyagents::harness::cost`. The current local
seams are crate-internal `tinyagents/middleware.rs::CostBudgetMiddleware`
(inside the 1861-line middleware module), crate-internal
`agent/harness/turn_subagent_usage.rs` (176) for task-local parent-turn rollup,
and crate-internal `agent/cost.rs::TurnCost` for the web footer payload, budget
stop hooks, and legacy progress compatibility.

## Steps

1. **Normalize records:** carry `cached_input_tokens`, cache-creation tokens,
   `reasoning_tokens` (crate `Usage` has it), image/audio/embedding usage where
   providers report; add `cost_source: estimated | provider_charged` to
   `TokenUsage`; keep provider-charged USD via an out-of-band carry (crate
   `Usage` has no USD field — residual upstream gap). Current `TokenUsage`
   persists cached-input provenance and `cost_source`; reasoning and
   cache-creation tokens remain zero until providers report them.
2. **Crate budget middleware:** replace bespoke `CostBudgetMiddleware`
   daily/monthly pre-checks with `BudgetMiddleware` where limits map; add
   per-run and per-thread budgets (new `CostConfig` fields + thread-id
   threading). TinyAgents `BudgetTracker` is run/tree-local, so it does not
   directly replace OpenHuman's persistent daily/monthly `CostTracker`
   semantics. Preflight `BudgetExceeded { blocked: true }` fails a model call
   before dispatch; post-spend `BudgetExceeded { blocked: false }` is only an
   event unless OpenHuman installs an explicit pause/failure policy. 1.3.0 also
   emits `BudgetReserved`/`BudgetReconciled`, but current preflight estimates
   input tokens and blocks against configured limits rather than pre-reserving
   cached tokens or projected USD for the next provider call. First build an
   OpenHuman limits adapter and define `UsageRecorded` ownership so the bridge,
   crate accounting middleware, and unobserved-turn fallback cannot record the
   same model call twice.
3. **Lineage rollup:** stamp cost records with `run_id`/`root_run_id` from
   the observation stream (needs `TokenUsage` schema fields); parent totals
   via `UsageTotals`/`ChildRun` instead of the `turn_subagent_usage`
   task-local where lineage suffices; keep the task-local only for detached
   children the tree can't see (or move them to TaskStore rollup — 07.2).
4. **Embedding usage:** embedding calls record usage/cost with provider,
   model, dimensions, vector count (ties into 09).

## Deletions

- Later only: crate-internal `CostBudgetMiddleware` after crate budget mapping
  covers OpenHuman daily/monthly stop behavior, `agent/cost.rs::TurnCost` only
  where `UsageTotals` covers all current web-footer, stop-hook, TokenJuice, and
  audit callers, and `turn_subagent_usage.rs` only after 07.2 rollup parity. Do
  not delete any of these until duplicate `UsageRecorded` semantics are resolved
  and run-tree/thread-budget integration covers the current stop-hook and
  parent-turn rollup paths.

## Acceptance

- Dashboard totals identical before/after on a scripted multi-child turn
  (no double count); budget-exceeded stops a recursive run pre-spend.
