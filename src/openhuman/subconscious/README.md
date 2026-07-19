# subconscious

The subconscious is OpenHuman's **Deep Reflection Layer**: an offline, cron-driven
loop that consumes a compressed view of how a *world* changed and emits short,
dense outputs that steer the rest of the system. It is a **factory** — one generic
reflection runner instantiated once per world:

- **`memory`** — the user's connected memory sources (Gmail / Slack / Notion /
  folders). Observes a `memory_diff` against a baseline checkpoint; reflects with
  the slim decision agent (to-dos, goals, `notify_user`, delegation).
- **`tinyplace`** — the tiny.place orchestration world. Observes the 20:1-compressed
  execution history + cumulative world-state diff; reflects with a **tool-free**
  steering synthesis that emits `STEERING_DIRECTIVE`s for the reasoning core.

Adding a world is a new profile file + one factory arm — not another engine.

## The generic tick (`instance.rs`)

Each tick body is a [`tinyagents` `CompiledGraph`](../tinyagents) — the same durable
runtime the orchestration wake path runs on:

```text
START ─► observe ─┬─(quiet)──────────────────────────► commit ─► done ─► END
                  └─(changed)─► prepare ─► reflect ─┬─(ok)──► commit ─► done ─► END
                                                    └─(err)────────────► done ─► END
```

The node handlers delegate 1:1 to the world's [`SubconsciousProfile`](profile.rs) —
the profile **is** the injected runtime. Everything world-agnostic lives in the
runner **once**, so every world gets it for free:

- per-instance tick lock + 5s acquisition skip,
- generation/supersede counter (checked in the `commit` node **and** post-run, so a
  superseded tick never advances state or commits),
- `TICK_TIMEOUT` (30 min) wall clock,
- provider gate + per-instance rate-cap halt (signature `"<id>|<provider-sig>"`, so
  one world's 413/TPM halt never silences another — TAURI-RUST-HXF),
- tool-capability classifier (TAURI-RUST-ADC),
- advance-baseline-only-on-success, quiet-tick short-circuit,
- `SqliteCheckpointer` resume of an interrupted tick.

## Profiles (`profiles/`)

A profile implements `observe → prepare_context → reflect → commit` + `origin`:

| Method | `memory` | `tinyplace` |
| --- | --- | --- |
| `observe` | diff connected sources vs the baseline checkpoint | load the unreviewed compressed history + world-diff window |
| `prepare_context` | read-only `context_scout` over the diff | none (steering is deliberately tool-free) |
| `reflect` | slim decision agent (`hint:subconscious`, Full autonomy) → `Acted` | tool-free provider chat → `Steered`/`Idle` |
| `commit` | re-checkpoint the world baseline | advance the review cursor to the observed window |
| `origin` | tainted iff the diff carried external content | always `SubconsciousTainted` |

## Persistence

SQLite at `<workspace>/subconscious/subconscious.db` (per-user workspace). State
keys are **namespaced per instance** (`"<instance>:<key>"`):

- `subconscious_state` — REAL KV: `memory:last_tick_at`, `tinyplace:last_tick_at`.
- `subconscious_state_text` — TEXT KV: `memory:baseline_checkpoint_id`.
- The tinyplace **review cursor** lives in the orchestration store, not here — the
  profile owns it via `commit`.
- Legacy single-engine keys (`last_tick_at`, `baseline_checkpoint_id`) migrate to
  the `memory:`-namespace via an idempotent, old-version-tolerant UPDATE in the DDL
  batch.
- Tick graph checkpoints: `<workspace>/subconscious/graph_checkpoints.db`, thread
  `subconscious:<instance>:<tick_id>`.

Legacy task/log/escalation/reflection tables are retained for back-compat with
existing DBs but are no longer written or read.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused; re-exports the factory, instance, profile types, session, source_chunk, schemas. |
| `profile.rs` | `SubconsciousProfile` trait + serde `Observation`/`Reflection`. |
| `instance.rs` | `SubconsciousInstance` — the generic tinyagents-graph runner + scheduler/circuit-breaker shell + `status()`/`is_due()`. |
| `profiles/memory.rs` | `MemoryProfile` + `memory_instance()` — the memory world (world-diff render, decision agent, baseline). |
| `profiles/tinyplace.rs` | `TinyPlaceProfile` + `tinyplace_instance()` — orchestration steering (wraps `orchestration::ops::load_review_window` / `synthesize_and_persist`). |
| `provider.rs` | Shared subconscious provider routing, rate-cap halt signature, and permanent-error classifiers (world-agnostic). |
| `factory.rs` | `SubconsciousKind` + `make_subconscious` + `enabled_kinds` — the bootstrap set. |
| `registry.rs` | Keyed `HashMap<Kind, Arc<SubconsciousInstance>>`; `get_or_init_instance`, `registered_instances`, `bootstrap_after_login`, user-switch reset. Spawns the heartbeat loop + opt-in trigger orchestrator. |
| `heartbeat/` | Periodic scheduler + event planner; each interval fans out, ticking every registered instance whose cadence has elapsed, concurrently. |
| `store.rs` | SQLite persistence + DDL; instance-namespaced `get/set_last_tick_at`, `get/set_baseline_checkpoint_id` + legacy-key migration. |
| `schemas.rs` | RPC controllers (`subconscious.status` / `subconscious.trigger`). |
| `session.rs`, `source_chunk.rs`, `user_thread.rs`, `agent/` | Unchanged roles (opt-in trigger session, chunk hydration, `notify_user`, the slim agent def). |

## RPC / controllers

Namespace `subconscious` (`openhuman.subconscious_<function>`):

| Function | Purpose |
| --- | --- |
| `status` | Legacy top-level fields mirror the **memory** instance; `instances[]` lists every registered world (each tagged with `instance`). Read entirely from SQLite / the small state mutex — never the tick lock. |
| `trigger` | Fire a tick for a world: optional `kind` (`"memory"` default, `"tinyplace"`, `"all"`). Spawned; returns immediately. |

## Notes / gotchas

- **Instances bootstrap post-login** (`registry::bootstrap_after_login`) against the
  per-user workspace. The heartbeat loop is the clock that drives every world's tick.
- **`status` never touches the tick mutex** — it reads SQLite + the small state
  mutex, since the tick lock is held for a full tick.
- **State only advances on success.** A failed reflect leaves `last_tick_at` and the
  baseline/cursor in place (re-observe the same window next tick). A superseded tick
  discards its result and skips commit.
- **Quiet ticks short-circuit before reflect** — no LLM call when `observe` is empty;
  the baseline is still refreshed via `commit`.
- **Taint:** the memory decision agent's toolset is internal-only, the tinyplace
  reflect path is a tool-free provider chat (source-scan test enforces no
  Agent/channel/send-message symbols); external effects stay gated by the approval
  gate. A tick that reacted to external content runs `SubconsciousTainted`.
- **One world's rate-cap halt never silences another** — the halt signature is
  prefixed with the instance id.
