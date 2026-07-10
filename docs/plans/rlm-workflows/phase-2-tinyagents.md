# Phase 2 — TinyAgents-side changes (separate PR)

Branch: `feat/repl-host-embedding` in `vendor/tinyagents` (repo
`tinyhumansai/tinyagents`). Raised as its **own PR**; the openhuman PR bumps
the submodule pointer to the merged commit.

Scope is intentionally minimal — only the host-embedding gaps identified in
Phase 1 that cannot be worked around from openhuman.

## 2.1 External cancellation (`ReplCancelFlag`)

**Problem:** a running cell can only be stopped by its wall-clock timeout.
OpenHuman needs to abort an in-flight RLM cell when the user cancels a run
(`workflows::run_log::cancel_run`) or the agent turn aborts.

**Change:** add a shared cancellation flag to the session:

- `repl/session/types.rs`: `pub struct ReplCancelFlag(Arc<AtomicBool>)` with
  `new() / cancel(&self) / is_cancelled(&self)`. Cheap `Clone`.
- `ReplSession::with_cancel_flag(flag)` stores it; `CellBuffers` carries it
  alongside `deadline`.
- Enforcement mirrors the deadline's two points, fail-closed:
  - the engine `on_progress` hook terminates the script with a
    `CANCELLED_TOKEN` sentinel → mapped to a new
    `TinyAgentsError::Cancelled` in `eval_cell`;
  - `bridge_block_on_raw` polls the flag in its timer race (tick the select
    with a short-interval timer when a flag is armed) so a blocked
    model/tool/agent call is dropped promptly on cancel.
- New error variant `TinyAgentsError::Cancelled(String)` in `src/error.rs`.

## 2.2 Live capability-call events on the `EventSink`

**Problem:** `ReplResult.calls` is only available after the cell completes;
long fan-outs look frozen in the UI.

**Change:** in `builtins/`, emit a typed event on `HostContext.events`
(the existing `EventSink` shared with the run context) at capability-call
start and completion — `AgentEvent::Custom`-style records carrying
`{session_id, kind: model|tool|agent|emit, name, elapsed?}`. The host
(openhuman) subscribes an `EventListener` and forwards to its own progress
sink. No new public types beyond what the event enum already supports; if
`AgentEvent` lacks a suitable variant, add `AgentEvent::ReplCall
{ record: ReplCallRecord, phase: Started|Completed }` behind the `repl`
feature.

## 2.3 Async-embedding documentation

Document at the `eval_cell` API surface (rustdoc + module README) that the
method blocks internally (`futures::executor::block_on`) and must be driven
via `spawn_blocking`/dedicated thread from async hosts. This is currently
only discoverable by reading `builtins/mod.rs`.

## 2.4 Tests (in tinyagents, alongside the changes)

TinyAgents' own convention requires tests with every behavior change, so
this crate does **not** defer them:

- cancel flag set before `eval_cell` → `Cancelled` without starting;
- cancel mid-script-loop → terminated promptly (`on_progress` path);
- cancel during a hanging capability future → dropped promptly
  (`bridge_block_on` path);
- events observed on the `EventSink` for `model_query`/`tool_call` start +
  completion with a `ScriptedModel`/`FakeTool`.

## Acceptance

`cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
cargo test --features repl` green in `vendor/tinyagents`; PR opened against
`tinyhumansai/tinyagents` with the summary + commands run.
