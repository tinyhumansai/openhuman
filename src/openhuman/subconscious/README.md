# subconscious

The subconscious is OpenHuman's background-awareness layer: a periodic loop that, on each tick, runs a small **structured three-stage flow** and lets a slim agent decide what (if anything) to act on. The actual periodic schedule is owned by the `heartbeat` domain; this module owns the tick flow, the world baseline, and the proactive output surface.

## The tick (`engine.rs`)

1. **memory_diff (code)** — diff the user's connected memory sources against the **world baseline** captured at the end of the previous tick (`memory_diff::ops::diff_since_checkpoint`). Renders a compact "what changed" summary. A quiet window (no changes), the first-ever tick, or a diff error short-circuits the tick: it refreshes the baseline, advances `last_tick_at`, and returns **without** running the agent — so idle ticks cost nothing.
2. **prepare_context (code)** — run the read-only `context_scout` over the diff (`agent_prepare_context::run_context_scout_with_catalog`, driven from engine code with the subconscious tool catalogue) to gather grounding from memory, goals/profile, integrations, and the web.
3. **decide (agent)** — hand `diff + prepared context` to the slim `subconscious` agent. It records/advances actionable follow-ups on the user's **global to-do board** (`update_task`, `threadId: "user-tasks"`), evolves **long-term goals** (`goals_*`), surfaces time-sensitive items (`notify_user`), or delegates deeper work (`spawn_async_subagent`).

Continuity across ticks lives in those durable stores (global to-dos + goals), not a bespoke scratchpad. The world baseline is the only per-tick engine state, persisted as a `memory_diff` checkpoint id.

## Responsibilities

- Route the decision turn to a local Ollama/LM Studio model or the OpenHuman cloud (`workload_local_model("subconscious")` / `subconscious_provider`). `run_agent` sets `default_model = "hint:subconscious"` so the session builder resolves the `subconscious` workload role (not `chat`); on the managed backend that pins the lightweight `chat-v1` tier.
- Run the decision agent with **Full** autonomy (it must write internal continuity — to-dos/goals/notify), while escalating taint via the turn origin: any tick that reacted to source changes runs as `SubconsciousTainted` so the approval gate refuses external-effect tools.
- Persist `last_tick_at` (status/dedupe) and `baseline_checkpoint_id` (the world snapshot the next tick diffs against) across restarts.
- Provide an overlap guard (generation counter) so a newer tick supersedes an in-flight one and discards its results without advancing state.
- Expose `status` / `trigger` over JSON-RPC.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused; re-exports the engine, session, source_chunk, schemas, and core types. |
| `types.rs` | `SubconsciousStatus`, `TickResult`. |
| `engine.rs` | `SubconsciousEngine` — the three-stage tick (`tick`/`tick_inner`), `prepare_context`, `refresh_baseline`, `run_agent`, world-diff rendering, provider routing (`resolve_subconscious_route`, `subconscious_provider_unavailable_reason`), `tick_origin_source`, tool-capability-error detection. |
| `store.rs` | SQLite persistence + DDL; `with_connection` with busy-timeout + retry (TAURI-RUST-A). `get/set_last_tick_at`, `get/set_baseline_checkpoint_id`. (Legacy task/log/escalation/reflection tables are retained for back-compat but no longer written.) |
| `source_chunk.rs` | `SourceChunk` + `resolve_chunks` / `parse_ref` — used by the agent prompt builder to hydrate reflection `source_refs` into frozen previews (`PREVIEW_MAX_CHARS = 400`). Shared with `agent::prompts`, not subconscious-only. |
| `session.rs` | `LongLivedSession` — persistent agent for the opt-in event-driven trigger path (`subconscious_triggers`); builds the `subconscious` agent and resumes history from the reserved orchestrator thread. |
| `user_thread.rs` | `NotifyUserTool` / `notify_user` — proactive user handoff; publishes `DomainEvent::ProactiveMessageRequested` and the reserved `subconscious:user` thread. |
| `agent/` | The slim `subconscious` agent definition: `agent.toml` (toolset) + `prompt.md`. |
| `global.rs` | Engine singleton: `get_or_init_engine`, `bootstrap_after_login`, `stop_heartbeat_loop`. Spawns the `heartbeat` loop and the opt-in trigger orchestrator. |
| `heartbeat/` | Periodic scheduler + event planner (meeting/reminder/notification delivery) that drives `engine.tick()`. |
| `schemas.rs` | RPC controller schemas + handlers (`subconscious.status` / `subconscious.trigger`). |
| `decision_log.rs`, `executor.rs` | Legacy stubs retained for back-compat; not on the live tick path. |

## Public surface

From `mod.rs`: `SubconsciousEngine`, `LongLivedSession`/`ProcessOutcome`/`ORCHESTRATOR_THREAD_ID`, `SourceChunk`, `SubconsciousStatus`/`TickResult`, `notify_user`/`NotifyUserTool`/`USER_THREAD_ID`, `all_subconscious_controller_schemas` / `all_subconscious_registered_controllers`, and the `global::*` lifecycle functions.

## RPC / controllers

Namespace `subconscious` (i.e. `openhuman.subconscious_<function>`):

| Function | Purpose |
| --- | --- |
| `status` | Engine status (read entirely from DB to avoid blocking on the tick mutex). |
| `trigger` | Manually fire a tick (spawned in the background; returns immediately). |

## Agent tools

This module owns `user_thread.rs` (`notify_user`). The tick's other tools come from elsewhere: `memory_diff` (`memory_diff` domain), `agent_prepare_context` / `spawn_async_subagent` (`agent_orchestration`), `update_task` (`todos`/agent tools), `goals_*` (`memory_goals`).

## Persistence

SQLite at `<workspace>/subconscious/subconscious.db` (per-user workspace):
- `subconscious_state` — REAL KV holding `last_tick_at` (restart-durable dedupe cutoff).
- `subconscious_state_text` — TEXT KV holding `baseline_checkpoint_id` (the `memory_diff` checkpoint the next tick diffs against).
- Legacy tables (`subconscious_tasks` / `_log` / `_escalations` / `_reflections` / `_hotness_snapshots`) are retained for back-compat with existing DBs and are no longer written or read.

The world snapshots/checkpoints themselves live in the `memory_diff` domain's own DB (`<workspace>/memory_diff/diff.db`), not here.

`with_connection` runs all DDL on every open, with a 5s busy timeout and 3-retry exponential backoff for transient `SQLITE_BUSY`/`SQLITE_LOCKED`.

## Dependencies

- `openhuman::config` — `Config`/`HeartbeatConfig`, provider routing, `workspace_dir`.
- `openhuman::memory_diff` — `ops::diff_since_checkpoint` / `ops::create_checkpoint` + diff types (stage 1 + baseline).
- `openhuman::agent_orchestration` — `run_context_scout_with_catalog` (stage 2).
- `openhuman::agent` — `Agent`, turn-origin taint plumbing (stage 3).
- `openhuman::heartbeat` — `HeartbeatEngine`; `global.rs` spawns the periodic loop that calls `tick`.
- `openhuman::credentials` — `AuthService`/`APP_SESSION_PROVIDER` for cloud-session provider availability.
- `openhuman::scheduler_gate` — `is_signed_out()` gate for the cloud provider.

## Notes / gotchas

- **Engine bootstraps post-login** (`global::bootstrap_after_login`) so state writes to the per-user workspace, not the pre-login global default.
- **`status` RPC never touches the engine mutex** — it reads straight from SQLite, since the engine lock is held for the full tick.
- **State only advances on success.** A failed decision turn leaves `last_tick_at` and the baseline in place, so the next tick re-diffs the same window instead of losing it. A superseded tick discards its result.
- **Quiet ticks short-circuit before the agent** — if the diff has no changes, no decision turn runs (and no cost is incurred); the baseline is still refreshed.
- **Taint:** any tick that reacted to source changes runs `SubconsciousTainted`; the decision agent's slim toolset is internal-only, and external effects (incl. inside delegated work) stay gated by the approval gate.
