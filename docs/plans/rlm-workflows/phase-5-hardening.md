# Phase 5 — Error handling, timeouts, cancellation, observability

The RLM tool executes model-authored code; every failure mode must be
fail-closed, bounded, and reported back to the model as a *usable* tool
result (so it can fix its script), never as a panic or a hung turn.

## 5.1 Error taxonomy → model-consumable results

| Failure | Source | Returned tool result |
| ------- | ------ | -------------------- |
| Script parse/runtime error | `TinyAgentsError::Validation` | `is_error=true`, rhai diagnostic verbatim + hint ("fix the script and retry in the same session") |
| Wall-clock timeout | `TinyAgentsError::Timeout` | which phase timed out (script vs in-flight capability call), elapsed, configured limit |
| Op/output/script-size/call-count limit | `LimitExceeded` | the specific limit + counter values + how to split work across cells |
| Unknown capability | `ModelNotFound`/`ToolNotFound`/`Capability` | the bad name + the live list of registered tool/agent/model names |
| Recursion depth | `SubAgentDepth` | depth + max |
| User cancel | `Cancelled` (new, Phase 2) | "cancelled by user", partial `calls` summary |
| Session busy / evicted / unknown `session_id` | rlm domain | typed message; unknown id ⇒ fresh session created and noted |
| spawn_blocking join error / poisoned session | rlm domain | session is dropped (poisoned namespaces are never reused), error reported |

Partial results: on timeout/cancel/limit, include whatever `CellBuffers`
captured (stdout so far is not available from a failed `eval_cell` in v1 —
note this; calls recorded before the failure are, via the live event
stream).

## 5.2 Layered time bounds (belt and braces)

1. rhai `on_progress` deadline — pure script loops.
2. `bridge_block_on` timer race — hung capability futures.
3. Outer `tokio::time::timeout(policy.timeout + 5s)` around
   `spawn_blocking` — defends against bugs in 1–2; logs at `error` if it
   ever fires (it should not), and drops the session entry (the blocking
   thread may still be unwinding; never reuse it).
4. Harness-level `ToolTimeout::Secs` — the final backstop; the tool's own
   timeout is always set below it.

## 5.3 Cancellation flow

user cancel / turn abort
→ existing run-cancellation context (`tinyagents/run_cancellation_context.rs`)
→ `ReplCancelFlag::cancel()`
→ script terminated at next statement OR in-flight capability future dropped
→ `Cancelled` tool result → session left intact (resumable).

## 5.4 Resource-exhaustion guards beyond ReplPolicy

- Session manager LRU cap + idle TTL (Phase 3) — no unbounded namespaces.
- Result truncation via `max_result_size_chars` before returning to the
  model.
- `calls` summarized (kind, name, elapsed, ok/err) — never raw payloads —
  in the tool result; full detail only at `debug` log level.
- Nested `rlm` is impossible (excluded from the bridge) — no REPL-in-REPL.

## 5.5 Observability (per debug-logging rules)

- `[rlm]` tracing on: session create/evict/close, policy resolution, cell
  start/end (elapsed, counters), every capability call start/finish (name,
  elapsed, ok/err), every error mapping, cancellation, both timeout layers.
- `AgentProgress` events for live UI: cell started, capability calls
  (forwarded from EventSink), cell completed.
- `DomainEvent` bus: reuse the `tool` category events already published by
  `ToolAdapter` for inner calls; add coarse start/finish workflow events for
  the cell itself.
- Langfuse: inner model/agent calls already traced through the existing
  provider/observability path; tag spans with `rlm.session_id`.
- Never log script-embedded secrets: scripts are logged elided (first line,
  hash, byte size) at `info`; full script only at `trace`.
