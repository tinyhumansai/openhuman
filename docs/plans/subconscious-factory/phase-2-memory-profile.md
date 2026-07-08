# Phase 2 — Extract the memory profile (behavior-identical)

Goal: move today's stages 1–3 out of `engine.rs` into
`profiles/memory.rs`, switch the live engine to
`SubconsciousInstance::new(MemoryProfile, config)`, and delete the old
monolithic `tick_inner`. **No behavior change** — this phase is a pure
extraction, verified by keeping `engine_tests.rs` green against the new
composition.

## 2.1 What moves where

| Today (`engine.rs`) | Destination |
| --- | --- |
| baseline load + `diff_since_checkpoint` + `world_diff_change_count` | `MemoryProfile::observe` |
| `render_world_diff` + `MAX_ITEMS_PER_SOURCE` | `profiles/memory.rs` (private) |
| `prepare_context` (context_scout + `with_root_parent`, TAURI-RUST-HMW) | `MemoryProfile::prepare_context` |
| `SUBCONSCIOUS_TOOL_CATALOG` | `profiles/memory.rs` |
| `run_agent` (slim agent, `hint:subconscious`, Full autonomy, mode → iteration caps, user-message contract) | `MemoryProfile::reflect` |
| `refresh_baseline` (`create_checkpoint` + persist id) | `MemoryProfile::commit` |
| `tick_origin_source` | `MemoryProfile::origin` (keep the free fn for its unit test) |
| **the inline `run_orchestration_review` call (stage 0)** | **stays temporarily** — moved in phase 3 |

Note on the stage-0 call: during phase 2 it lives in a small shim at the top
of the runner (`if profile.id() == "memory" { run_orchestration_review(...) }`)
or equivalently stays as a pre-hook closure passed by the bootstrap. Ugly on
purpose and clearly marked `// phase-3 removes this`; it keeps phase 2 purely
mechanical.

## 2.2 `MemoryProfile` specifics

- `id()` → `"memory"`; `cadence` → `mode.default_interval_minutes().max(5)`
  minutes (today's value).
- `observe`: returns `has_changes == false` for first-tick/no-baseline, quiet
  window, or diff error (each with today's log lines). Sets
  `has_external_content = true` whenever there are changes (every change
  originates from a source sync — today's comment carries over).
- `reflect`: returns `Reflection::Acted { response_chars }`; the SubconsciousMode
  → `max_tool_iterations` mapping (15 / 30 / Off→short-circuit) is unchanged.
- `commit`: re-checkpoint + persist under the namespaced key
  (`memory:baseline_checkpoint_id`). Best-effort, warn on failure (unchanged).
- `origin`: `tick_origin_source(obs.has_external_content)`.

## 2.3 Public-surface continuity

`mod.rs` keeps exporting `SubconsciousEngine` (alias), `SubconsciousStatus`,
`TickResult`, `notify_user`, session/source_chunk items — nothing outside the
domain changes in this phase. `global.rs` swaps its construction call only.

## 2.4 Tests

- Port `engine_tests.rs` to construct `SubconsciousInstance` with
  `MemoryProfile` — assertions unchanged (rate-cap halt lifecycle, provider
  routing/signature, origin escalation, tool-capability detection).
- New: `MemoryProfile::observe` unit tests over a seeded memory_diff store
  (quiet vs changed windows), reusing whatever fixture `memory_diff` tests use.
- The agent-toolset isolation test in `orchestration/ops.rs`
  (`subconscious_agent_tool_surface_has_no_channel_or_effect_tools`) keeps
  compiling — `agent/agent.toml` does not move.
