# 05.2 — One event bridge; delete engine/

Two progress paths remain: `OpenhumanEventBridge`
(`tinyagents/observability.rs`) and scattered direct `publish_global` calls in
session/turn code. The old shared `harness/engine/` progress/checkpoint seams
are gone.

## Steps

1. Done for the reusable engine module: `ProgressReporter`/`TurnProgress` moved
   under `session/tool_progress.rs`, leaving no shared `harness/engine/`
   surface. The old session-local `agent_tool_exec` compatibility shim is
   deleted.
2. Done: `engine/checkpoint.rs` is gone. `CapPauser` owns the graceful
   max-iteration stop, and the remaining sub-agent checkpoint summary is
   localized in `subagent_runner/ops/checkpoint.rs`.
3. Sweep direct agent-run/tool/subagent lifecycle `DomainEvent` publishes on
   turn paths (session/turn, orchestration tools) — emit
   through the bridge or a typed helper so ordering/rate-limiting is
   single-owner.
4. Current finding (2026-07-02): `agent/progress_tracing.rs` (722) plus
   `agent/progress_tracing/tests.rs` (616) are not currently redundant. The
   root module is the opt-in `observability.agent_tracing` exporter fed by the
   web progress bridge's `AgentProgress` stream. Retain it until the 05.1
   journal path can produce the same metadata-only OTel/Langfuse spans and
   append/export contract.

## Deletions

- Deleted: `src/openhuman/agent/harness/engine/` (checkpoint + progress seams).
- Redundant publishes found in step 3.
- Retain `agent/progress_tracing.rs` + sibling tests for now; delete only after
  journal-backed span export proves parity with the current config contract.

## Acceptance

- Progress-event parity fixture: same `AgentProgress` sequence for a
  scripted turn before/after (tool timeline, thinking, cost footer).
- No `engine::` imports remain.
