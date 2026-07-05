# Subconscious factory — one reflection engine, many worlds

Redesign `src/openhuman/subconscious/` around the split-brain spec
(`state (1).md`, "Autonomous Closed-Loop LangGraph Harness"): the subconscious
is the **Deep Reflection Layer** — an offline, cron-driven loop that consumes a
compressed view of how a world changed and emits short, dense outputs that
steer the rest of the system. Today that layer exists twice, fused together
inside one engine. This plan pulls it apart into a **factory** that can
instantiate a subconscious per *world*:

- **`memory`** — OpenHuman's internal high-level world: the user's connected
  memory sources (Gmail/Slack/Notion/folders). Observes a `memory_diff`
  against a baseline checkpoint; reflects with the slim decision agent
  (to-dos, goals, `notify_user`, delegation).
- **`tinyplace`** — the tiny.place orchestration world: harness-session
  interactions flowing through the `orchestration` domain. Observes the
  20:1 compressed execution history + cumulative world-state diff; reflects
  with a tool-free steering synthesis that emits `STEERING_DIRECTIVE`s for the
  reasoning core.

New kinds (e.g. a per-team world, a channels world) become one new profile
file, not another engine.

## Where we are today

| Spec concept (state (1).md) | Existing implementation |
| --- | --- |
| Quick LLM / Front-End Agent | `orchestration/frontend_agent/` (`hint:chat`, two-pass) |
| Reasoning LLM / orchestration core | `orchestration/reasoning_agent/` (`hint:reasoning`), wake graph in `orchestration/graph/` |
| Bi-directional loop routing | `orchestration/graph/build.rs` + `ops::invoke_with_runtime` (debounce, idempotence cursor, dm_sent latch) |
| 20:1 compression engine | `orchestration/graph/compress.rs` + `ProductionRuntime::compress` |
| Cumulative world-state diff | `orchestration/graph/world_diff.rs` + `store::append_world_diff` |
| Context lifecycle hooks (80–90%) | `context_guard`/`evict` nodes → memory-RAG eviction |
| Subconscious LLM / steering | `orchestration/steering.rs` + `ops::run_orchestration_review` — **but invoked inline from `subconscious::engine::tick_inner`** |
| Cron trigger | `subconscious/heartbeat/` driving `SubconsciousEngine::tick` |

The problem is confined to `subconscious/engine.rs`: `tick_inner` is a
hard-wired composite — stage 0 calls `orchestration::ops::run_orchestration_review`
(the tiny.place world), then stages 1–3 run the memory world (memory_diff →
context scout → decision agent). One tick lock, one circuit breaker, one
status object, one baseline store — for two unrelated worlds. Neither can get
its own cadence, provider signature, halt state, or status, and a third world
cannot be added without growing the composite further.

## Target shape

```
subconscious/
├── mod.rs               exports (unchanged public names + new factory surface)
├── profile.rs           SubconsciousProfile trait + Observation/Reflection types
├── engine.rs            generic SubconsciousInstance: tick graph (tinyagents CompiledGraph) + scheduler shell
├── factory.rs           SubconsciousKind + make_subconscious(kind, config)
├── registry.rs          (grown from global.rs) kind → instance map, lifecycle
├── profiles/
│   ├── memory.rs        today's stages 1–3, extracted verbatim
│   └── tinyplace.rs     wraps orchestration::ops::run_orchestration_review
├── store.rs             instance-namespaced KV + legacy-key migration
├── heartbeat/           unchanged scheduler; drives every registered instance
├── agent/               unchanged (memory profile's decision agent)
├── session.rs, user_thread.rs, source_chunk.rs, schemas.rs   unchanged roles
```

The generic tick is a **`tinyagents` `CompiledGraph`** (the same runtime the
orchestration wake path runs on — see phase 6), wrapped by scheduler concerns
in `engine.rs`, identical for every kind:

```
lock → timeout guard → config load → provider gate + rate-cap halt (per-instance signature)
  → run graph:  START ─► observe ─┬─(quiet)──────────────────────► commit ─► END
                                  └─► prepare_context ─► reflect ─► commit ─► END
      · nodes delegate to the profile (the profile IS the injected runtime)
      · SqliteCheckpointer → a killed tick resumes instead of losing its window
  → superseded-generation check
  → per-instance state/status update
```

Everything that is currently *good* about the engine — tick lock, generation
counter, `TICK_TIMEOUT`, rate-cap circuit breaker (TAURI-RUST-HXF), tool-capability
detection (TAURI-RUST-ADC), only-advance-on-success baselines, quiet-tick
short-circuit — moves into the generic runner **once** and every world gets it
for free.

## The factory

```rust
pub enum SubconsciousKind { Memory, TinyPlace }

pub fn make_subconscious(kind: SubconsciousKind, config: &Config) -> SubconsciousInstance {
    let profile: Arc<dyn SubconsciousProfile> = match kind {
        SubconsciousKind::Memory    => Arc::new(profiles::memory::MemoryProfile::new(config)),
        SubconsciousKind::TinyPlace => Arc::new(profiles::tinyplace::TinyPlaceProfile::new(config)),
    };
    SubconsciousInstance::new(profile, config)
}
```

`registry.rs` (today's `global.rs`) holds the instances, bootstraps the enabled
set after login (`Memory` when `heartbeat.enabled`; `TinyPlace` when
`orchestration.enabled`), and tears all of them down on user switch.

## Invariants that must survive the refactor

1. **Isolation** — the subconscious never contacts anyone. The tinyplace
   profile stays a tool-free provider chat; the memory profile's agent toolset
   keeps the no-channel/no-outbound test (`subconscious_agent_tool_surface_has_no_channel_or_effect_tools`).
2. **Taint** — any tick that reacted to external content runs
   `SubconsciousTainted` (memory: diff-driven ticks; tinyplace: always).
3. **State only advances on success**; a superseded tick discards its result.
4. **Quiet ticks cost nothing** — no LLM call when `observe()` is empty.
5. **`subconscious.status` never touches the tick mutex** — reads from SQLite.
6. **Back-compat** — existing DB keys (`last_tick_at`, `baseline_checkpoint_id`)
   migrate to the `memory:`-namespaced keys; the RPC keeps its legacy top-level
   fields mirroring the memory instance.

## Phases

| Phase | File | Deliverable |
| --- | --- | --- |
| 1 | [phase-1-profile-and-engine.md](phase-1-profile-and-engine.md) | `SubconsciousProfile` trait, generic `SubconsciousInstance`, namespaced store |
| 2 | [phase-2-memory-profile.md](phase-2-memory-profile.md) | Extract the memory flow into `profiles/memory.rs`, behavior-identical |
| 3 | [phase-3-tinyplace-profile.md](phase-3-tinyplace-profile.md) | Wrap the orchestration review as `profiles/tinyplace.rs`; delete the inline call |
| 4 | [phase-4-factory-registry-rpc.md](phase-4-factory-registry-rpc.md) | Factory + registry lifecycle, heartbeat fan-out, per-instance RPC |
| 5 | [phase-5-tests-and-docs.md](phase-5-tests-and-docs.md) | Test matrix, migration tests, README/docs updates, rollout |
| 6 | [phase-6-tinyagents-reuse.md](phase-6-tinyagents-reuse.md) | TinyAgents graph reuse map + upstream PRs to `tinyhumansai/tinyagents` (deadline, cancel token, checkpoint GC) |
| 7 | [phase-7-ui.md](phase-7-ui.md) | UI: both kinds visible + triggerable — instance cards in the Subconscious tab, steering header on the orchestration tab's Subconscious window |

Each phase is a separately compilable, committable slice; phases 2 and 3 are
pure extractions (no behavior change), which keeps the diffs reviewable and
the coverage gate satisfiable per-slice.
