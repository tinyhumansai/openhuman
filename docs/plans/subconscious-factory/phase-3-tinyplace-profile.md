# Phase 3 — The tiny.place profile (orchestration steering)

Goal: make the tiny.place/orchestration subconscious a first-class instance
and delete the stage-0 shim from the runner. After this phase the two worlds
tick independently: separate locks, cadences, circuit breakers, failure
counters, and status rows.

## 3.1 `profiles/tinyplace.rs`

Wraps the machinery that already exists in `orchestration/`:

- `id()` → `"tinyplace"`.
- `cadence`: its own knob (see 3.3); default = the heartbeat interval, so the
  merged behavior matches today (review ran once per memory tick).
- `observe`: the load half of `run_orchestration_review` —
  `review_cursor` → `list_unreviewed_compressed(REVIEW_BATCH)` +
  `list_recent_world_mutations(REVIEW_BATCH)` + `current_cycle_counter`.
  `has_changes = !compressed.is_empty()` (today's idle gate).
  `rendered = build_steering_prompt(&summaries, &mutations)`.
  `commit_token = newest reviewed created_at`.
  `has_external_content = true` always — harness DMs are third-party content.
- `prepare_context`: default no-op (steering is deliberately tool-free; no
  scout).
- `reflect`: the synth half — `synthesize_steering` (tool-free
  `create_chat_provider("subconscious")` chat, `SubconsciousTainted` origin,
  one retry on contract violation) + persist
  (`insert_steering_directive`, supersede prior, `record_subconscious_directive`
  into the local Subconscious window + event publish). Returns
  `Steered { directive_id }` or `Idle` (clean `NONE` / twice-failed —
  today both still advance the cursor; preserve that by returning `Idle`,
  not `Err`, for those cases).
- `commit`: `set_review_cursor(commit_token)`. This is where the phase pays
  off: today the cursor advance is tangled into `run_orchestration_review`'s
  three exit paths; splitting observe/reflect/commit lets the runner enforce
  "advance only when the tick wasn't superseded" uniformly.
- `origin`: always `SubconsciousTainted`.

## 3.2 Refactor of `orchestration::ops`

`run_orchestration_review` is currently the public entry called by the
subconscious engine. Plan:

- Split it into store-facing pieces the profile calls
  (`load_review_window(config) -> ReviewWindow`,
  `synthesize_and_persist(config, window, tick_id) -> Option<i64>`), keeping
  them in `orchestration::ops` — the orchestration domain still owns its
  store shapes and the steering contract; the subconscious profile is just
  the *scheduler + policy* around them.
- Keep a thin `run_orchestration_review` wrapper for the two existing tests
  (or port those tests to drive the profile; implementor's choice — the
  invariants asserted must survive: emit-then-inject-next-cycle, idempotent
  re-tick, exactly-one-directive).
- Delete the stage-0 shim in the subconscious runner (`// phase-3 removes
  this` marker from phase 2).

## 3.3 Config

- `orchestration.enabled` remains the master gate for the tinyplace instance
  (profile `observe` returns quiet when disabled — same as today's early
  `Ok(false)`).
- Add `orchestration.review_interval_minutes: Option<u32>` (None = heartbeat
  interval) consumed by `cadence`. Schema change in
  `config/schema/orchestration.rs` + env override in `load.rs` +
  `.env.example` note.

## 3.4 Isolation invariants (unchanged, now testable per-instance)

- The tinyplace reflect path constructs **no Agent and no toolset** — it is a
  provider chat. Add a compile-level guard mirroring the existing agent-toml
  test: the profile module must not import `tinyplace::agent_tools` or any
  `send_message`-family symbol (source-scan test like
  `orchestration_logs_never_reference_message_bodies`).
- The steering directive remains an out-of-band writer: it is read by
  `apply_cycle_steering` at the next wake, never edges into the graph.

## 3.5 Tests

- Profile-level: seeded orchestration store → observe has_changes; scripted
  provider (existing `test_provider_override`) → `Steered`; cursor advanced
  via runner commit; re-tick idle.
- Runner-level: tinyplace + memory instances ticking concurrently against one
  workspace — no lock contention between them, independent `last_tick_at`
  keys, one instance's rate-cap halt does not gate the other.
- Regression: the full-cycle orchestration graph test keeps passing (steering
  injection path untouched).
