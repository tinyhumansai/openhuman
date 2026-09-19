# learning

The agent **self-learning subsystem**. It observes completed turns, the memory substrate, and ingested transcripts; distils them into durable signals about the user (preferences, identity, vetoes, goals, tooling, communication style); and feeds those signals back into the agent's system prompt and `PROFILE.md`. The core abstraction is the **ambient personalization cache** (`user_profile_facets` table): a bounded, decaying set of scored `(class, key) → value` facets built by a stability detector from a stream of `LearningCandidate` evidence. The module also owns the one-shot LinkedIn enrichment pipeline and the transcript-to-memory ingestion pipeline.

The subsystem is organised in phases (issue #566): **Phase 1** the candidate taxonomy + buffer, **Phase 2** producers that emit candidates, **Phase 3** the stability detector + cache + scheduler, **Phase 4** prompt injection and `PROFILE.md` rendering.

## Responsibilities

- Collect evidence (`LearningCandidate`) from many producers into a thread-safe global ring-buffer.
- Periodically (and on relevant events) drain the buffer, score every `(class, key)` pair by a recency-decayed stability formula, resolve value conflicts, enforce per-class budgets, assign lifecycle states, and persist the result to `user_profile_facets`.
- Run post-turn hooks: reflect on turns (`ReflectionHook`), track tool effectiveness (`ToolTrackerHook`), and extract explicit user preferences (`UserProfileHook`).
- Inject learned context, the user profile, and a memory-access instruction into the system prompt; render cache-derived managed blocks into `PROFILE.md`.
- Expose RPC controllers for inspecting / managing the cache and facets, plus running LinkedIn enrichment and saving a profile.
- Mine Gmail (via Composio) for a LinkedIn URL, scrape it via Apify, summarise it, and persist `PROFILE.md` + memory.
- Ingest completed session transcripts into durable conversational memory + reflections.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused module root; phase docstrings + `pub use` re-exports. |
| `startup.rs` | Registers the always-on Phase 2/3/4 subscribers (signature producer, rebuild trigger + 30-min loop, `ProfileMdRenderer`) on the global event bus; idempotent, `OnceLock`-guarded (#5003). |
| `candidate.rs` | Phase 1 taxonomy re-exported from the contract crate (`tinymemory_api::learning::{FacetClass, CueFamily, LearningCandidate}`, `tinymemory_api::host::EvidenceRef`), plus the bounded FIFO `Buffer` with a `global()` singleton (cap 1024). |
| `cache.rs` | `FacetCache` — typed wrapper over `user_profile_facets`; class↔key helpers (`class_from_key`, `key_with_class`, `class_prefix`). Delegates to the `MemoryProfile` family via `crate::memory::guard::MemoryGuard`. |
| `stability_detector.rs` | Phase 3 `StabilityDetector::rebuild` — the scoring/budget/state-assignment cycle; thresholds, half-lives, budgets, and the `stability()` formula. Publishes `CacheRebuilt`. |
| `scheduler.rs` | Periodic rebuild loop (`spawn_rebuild_loop`, default 30 min) + event-driven debounced trigger (`register_event_trigger`) subscribing to memory/tree-summarizer events. |
| `schemas.rs` + `schemas/` | `schemas.rs` holds the controller schema list and `RegisteredController` table; `schemas/profile_handlers.rs` has the `linkedin_enrichment` / `save_profile` / `rebuild_cache` / `cache_stats` handlers, `schemas/cache_helpers.rs` has the `get_cache()` helper, and `schemas/facet_handlers.rs` has the facet handlers (`list_facets`, `get_facet`, `update_facet`, `pin_facet`, `unpin_facet`, `forget_facet`, `reset_cache`) for the `learning.*` namespace. |
| `tools.rs` | The 11 `Learning*Tool`s mirroring the RPC surface (see "Agent tools" below). |
| `reflection.rs` | `ReflectionHook` post-turn hook: heuristic reflection-cue capture + LLM reflection, stores observations/patterns/preferences/reflections, emits Goal/Style candidates. |
| `tool_tracker.rs` | `ToolTrackerHook` post-turn hook + `ToolStats`; per-tool running success/failure/duration tallies in the `tool_effectiveness` memory category. |
| `user_profile.rs` | `UserProfileHook` post-turn hook; Aho-Corasick DFA over curated preference phrases, stores matches in the `user_profile` category. |
| `prompt_sections.rs` | `LearnedContextSection`, `UserProfileSection`, `MemoryAccessSection` (+ `MEMORY_ACCESS_INSTRUCTION`), `MemoryWriteSection` / `memory_write_instruction`, the memory tool-name constants + `any_tool_offered` (used by the harness to decide whether to inject the memory sections), and the async `load_learned_from_cache` (cache→prompt loader, cap 25). |
| `profile_md_renderer.rs` | `ProfileMdRenderer` — subscribes to `CacheRebuilt`, re-renders 5 managed `PROFILE.md` blocks (style/identity/tooling/vetoes/goals) from Active facets. |
| `linkedin_enrichment.rs` + `linkedin_enrichment/` | Gmail→LinkedIn→Apify enrichment pipeline: `linkedin_enrichment.rs` (`run_linkedin_enrichment`), `linkedin_enrichment/gmail_discovery.rs` (`scrape_linkedin_profile`), `linkedin_enrichment/profile_markdown.rs` (`summarise_profile_with_llm`, `render_profile_markdown`), and `linkedin_enrichment/memory_persistence.rs` (guard lookup + `put_profile_document` persistence). |
| `extract/` | Phase 2 producers (`mod.rs` + `signature.rs`, `heuristics.rs`, `summary_facets.rs`). |
| `extract/signature.rs` | Email-signature parser → Identity candidates; subscribes to `DocumentCanonicalized` (email). |
| `extract/heuristics.rs` | `LengthRatioDetector` / `EditWindowDetector` / `CorrectionRepeatDetector` → Style + Veto candidates via `record_turn`. |
| `extract/summary_facets.rs` | Parses the LLM summariser's structured JSON block; `route_facets_to_buffer` pushes validated candidates (requires `evidence_chunks`). |
| `transcript_ingest/` | Transcript→memory pipeline (`mod.rs`, `extract.rs`, `dedupe.rs`, `persist.rs`, `types.rs`). |
| `transcript_ingest/mod.rs` | `ingest_transcript_path` / `ingest_session_transcript`; extract→dedupe→persist into `conversation_memory` + `conversation_reflections` namespaces. |
| `test_profile.rs` | In-memory `MemoryProfile` test double (`HashMap` behind a mutex) mimicking the engine's facet ordering, for tests that don't need a real store. Not `#[cfg(test)]`-gated so integration tests under `tests/` (compiled without `cfg(test)`) can still use it. |
| `*_tests.rs` | Sibling test suites (`cache`, `candidate`, `reflection`, `prompt_sections`, `linkedin_enrichment`, `startup`, `tools`, `schemas`, …). |

## Public surface

Re-exported from `mod.rs`:

- **Candidate types**: `Buffer`, `CueFamily`, `EvidenceRef`, `FacetClass`, `LearningCandidate`.
- **Cache / detector**: `FacetCache`, `StabilityDetector`.
- **Hooks**: `ReflectionHook`, `ToolTrackerHook`, `UserProfileHook` (all impl `PostTurnHook`).
- **Prompt**: `LearnedContextSection`, `UserProfileSection`, `MemoryAccessSection`, `MemoryWriteSection`, `MEMORY_ACCESS_INSTRUCTION`, `memory_write_instruction`, `any_tool_offered`, `load_learned_from_cache`, and the tool-name constants `MEMORY_READ_TOOLS`, `MEMORY_WRITE_TOOLS`, `MEMORY_STORE_TOOL`, `MEMORY_WRITE_DELEGATE_TOOL`, `SAVE_PREFERENCE_TOOL`.
- **Profile**: `ProfileMdRenderer`.
- **Schemas**: `all_learning_controller_schemas`, `all_learning_registered_controllers`, `learning_schemas`.

Not re-exported but public within the module: `candidate::global()`, the `linkedin_enrichment::*` pipeline fns, `transcript_ingest::ingest_*`, `scheduler::{spawn_rebuild_loop, register_event_trigger, DEFAULT_REBUILD_INTERVAL}`, and `stability_detector` constants (`TAU_*`, `HALF_LIFE_*`, `BUDGET_*`, `stability`, `half_life`, `class_budget`).

## RPC / controllers

Namespace `learning` (wired into `crates/openhuman-core/src/core/all.rs`; 11 controllers). Methods:

| Method | Purpose |
| --- | --- |
| `learning.linkedin_enrichment` | Run the Gmail→LinkedIn→Apify pipeline (optional `profile_url` to skip Gmail search). |
| `learning.save_profile` | Write markdown to `{workspace_dir}/PROFILE.md`; optional `summarize` runs it through the LLM compressor first. |
| `learning.rebuild_cache` | Manually trigger a `StabilityDetector` rebuild; returns added/evicted/kept/total_size. |
| `learning.cache_stats` | Cache totals + per-state and per-class breakdown. |
| `learning.list_facets` | List Active + Provisional facets, optional `class` filter. |
| `learning.get_facet` | Fetch one facet by `class` + `key` suffix. Each returned facet carries its provenance (`evidence_refs`, `cue_families`) alongside the value/state fields. |
| `learning.update_facet` | Set a facet value and pin it (`user_state = Pinned`). |
| `learning.pin_facet` / `learning.unpin_facet` | Toggle `user_state` Pinned ↔ Auto. |
| `learning.forget_facet` | Mark `Dropped` + `user_state = Forgotten` (blocks re-promotion). |
| `learning.reset_cache` | Delete all `Auto` rows, preserve `Pinned`. |

Facet handlers build a `FacetCache` over `crate::memory::ops::guard::active_memory_guard()` (`get_cache()` in `schemas/cache_helpers.rs`); `linkedin_enrichment` / `save_profile` load config via `config::rpc::load_config_with_timeout`.

## Agent tools

`tools.rs` defines 11 LLM-callable tools mirroring the RPC surface above: `LearningListFacetsTool`, `LearningGetFacetTool`, `LearningCacheStatsTool`, `LearningUpdateFacetTool`, `LearningPinFacetTool`, `LearningUnpinFacetTool`, `LearningForgetFacetTool`, `LearningRebuildCacheTool`, `LearningResetCacheTool`, `LearningSaveProfileTool`, `LearningEnrichProfileTool`. Re-exported via `crates/openhuman-core/src/tools/mod.rs` (`pub use crate::agent::learning::tools::*;`). Separately, the `tool_effectiveness` stats this module writes are surfaced by the cross-cutting `tool_stats` tool in `crates/openhuman-core/src/tools/impl/system/tool_stats.rs`, and `memory_tools` references learning namespaces.

## Events

Uses the typed event bus (`crates/openhuman-core/src/core/bus.rs`, the process-wide `BUS` singleton; `core/events.rs` for `DomainEvent`; handlers implement `tinybus::EventHandler<DomainEvent>`):

- **Publishes**: `DomainEvent::CacheRebuilt { added, evicted, kept, total_size, rebuilt_at }` after each rebuild (`stability_detector.rs`).
- **Subscribes**:
  - `profile_md_renderer.rs` (`ProfileMdRenderer::subscribe`) → `CacheRebuilt` → re-render `PROFILE.md` blocks.
  - `scheduler.rs` (`RebuildTriggerHandler`, domains `memory` / `tree_summarizer`) → `DocumentCanonicalized` (email/document) and `TreeSummarizerPropagated` → debounced (60 s) rebuild.
  - `extract/signature.rs` → `DocumentCanonicalized` (email) → emit Identity candidates.

These are subscriber registrations rather than a single `bus.rs`; subscriptions must keep their `SubscriptionHandle` alive (callers store them in statics).

## Persistence

- **`user_profile_facets`** (the ambient cache) — accessed via `FacetCache`, which delegates to the `MemoryProfile` family of the active `MemoryGuard` (`list_active`, `list_all`, `get`, `upsert`, `set_user_state`, `delete`, `drop_below_threshold`; `reset_non_pinned` is a free fn in `cache.rs`). The SQL lives in the memory driver, not in this crate. Rows are `tinymemory_api::provider::ProfileFacet { key, value, state, user_state, stability, confidence, evidence_count, evidence_refs, class, cue_families, first/last_seen_at, … }`.
- **KV memory namespaces** (via the `Memory` trait): `learning_observations`, `learning_patterns`, `learning_reflections`, `user_profile`, `tool_effectiveness`, plus transcript-ingest `conversation_memory` / `conversation_reflections`. LinkedIn enrichment also upserts the scraped profile through the guard's documents family (`put_profile_document` in `linkedin_enrichment/memory_persistence.rs`) and writes `{workspace_dir}/PROFILE.md`.
- **In-memory**: the global `candidate::Buffer` (transient evidence, not persisted) and per-session state in `extract/heuristics.rs`.

## Dependencies

- `tinymemory_api` — `provider::{MemoryProfile, ProfileFacet, FacetState, FacetType, UserState}` behind `FacetCache`, and the candidate taxonomy re-exported by `candidate.rs` (`learning::{CueFamily, FacetClass, LearningCandidate}`, `host::EvidenceRef`).
- `crate::memory` — the `Memory` trait + `MemoryCategory` for KV persistence (hooks, transcript ingest), `guard::MemoryGuard` and `ops::guard::active_memory_guard` for the facet store, and `memory::api::provider::MemoryProvider` / `api::types` for the enrichment document upsert.
- `crate::agent::hooks` — `PostTurnHook` / `TurnContext` / `ToolCallRecord` implemented by the three hooks.
- `tinyagents_session::transcript` — `SessionTranscript` parsing for transcript ingestion.
- `crate::inference::provider::create_chat_model_from_string_with_model_id` (profile summarisation) and `crate::inference::host_runtime::global` (local reflection route); `ReflectionHook` also accepts an optional `tinyinference_llm::model::ChatModel` for the cloud fallback.
- `crate::config` — `Config` / `LearningConfig` / `ReflectionSource` feature flags and `config::rpc` loader.
- `crate::agent::prompts` — `PromptContext` / `PromptSection` / `LearnedContextData` for prompt injection.
- `crate::integrations::composio` — `composio::client` (Gmail fetch for enrichment) and `composio::profile_md` (`replace_managed_block` for `PROFILE.md`).
- `crate::integrations` — `build_client` / `IntegrationClient` for the Apify scrape call.
- `crate::core::bus` / `crate::core::events` / `tinybus` — `BUS.publish` / `BUS.subscribe`, `DomainEvent::CacheRebuilt`, and `tinybus::{EventHandler, SubscriptionHandle}`.
- `crate::core::all` / `crate::rpc` — controller registry types (`RegisteredController`, `ControllerFuture`) and `RpcOutcome` (the latter re-exported from the `openhuman-rpc` crate via `pub use openhuman_rpc as rpc` in `lib.rs`).

## Used by

- `crates/openhuman-core/src/core/all.rs` — registers the `learning.*` controllers + schemas.
- `crates/openhuman-core/src/agent/session_host/builder/factory.rs` (registers `LearnedContextSection` / `UserProfileSection` and the `ReflectionHook` when `config.learning.enabled`), `builder/helpers.rs` (`MemoryAccessSection` / `MemoryWriteSection` gated by `any_tool_offered`), `turn/context.rs` (reads the `learning_observations` / `learning_patterns` / reflections namespaces into `PromptContext.learned`), `turn/session_io/background_tasks.rs` (`transcript_ingest::ingest_transcript_path`), and `agent/tinyagents/host/learning_sink.rs` (`ToolTrackerHook` + `UserProfileHook` post-turn fan-out).
- `learning::startup::register_learning_subscribers` is the entry point that wires the Phase 2/3/4 subscribers; it is invoked from `crates/openhuman-core/src/core/jsonrpc.rs` (`register_domain_subscribers`, inside the `plan.agent` block — i.e. whenever the `DomainSet` allows `DomainGroup::Agent` — guarded by `learning_first_time()`), not from the skippable `channels::runtime::startup` path — see the "why" note in `startup.rs` (#5003).
- `crates/openhuman-core/src/modules/memory/people_chunks_retrieval.rs` (`impl MemoryProfile for ModuleMemoryProvider`, the driver `FacetCache` reaches through the guard), `integrations/composio/profile_md.rs` (`replace_managed_block`, used by `profile_md_renderer.rs`), `tools/impl/system/tool_stats.rs`, `tools/schemas.rs` — consume facet/learning types.

## Notes / gotchas

- **Pinned ⇒ stability ∞, Forgotten ⇒ stability 0.** User overrides hard-win over scoring in both `stability()` and `state_from_stability()`. `update_facet` implicitly pins.
- **Class is encoded in the key prefix** (`style/verbosity`, `goal/learn_rust`). `candidate.key` carries no prefix; the detector prepends `class_prefix`. Legacy rows without a recognised prefix are skipped by rebuild.
- **`emit_candidates_*` uses the global buffer length as a synthetic `episodic_id`** (a Phase-2 placeholder noted to be replaced by a real `episodic_log` row id).
- **Goal facets render value-only** (full sentence, no key prefix) in both prompt injection and `PROFILE.md`; other classes render `**key**: value`.
- **`load_learned_from_cache` is async** — the facet store sits behind the memory driver, so it is a driver call (it used to be a synchronous SQLite read) and degrades to empty on error. Both the cache path and the legacy KV-namespace path are still active (KV slated for later removal).
- **Reflection has a local/cloud gate.** The `ReflectionSource::Local` route requires `Config::workload_uses_local("learning")` (an `ollama:` `learning_provider`); if off it falls back to the optional cloud `ChatModel` or no-ops (empty string), and an empty response is a clean-skip sentinel that rolls back the throttle counter. Per-session reflections are throttled by `max_reflections_per_session`.
- **LinkedIn enrichment short-circuits** if `PROFILE.md` already exists, and the Composio-only Gmail-search stage is documented as currently erroring (Gmail-via-Composio removed) — callers should pass `preset_profile_url` obtained via the frontend's webview Gmail helper.
- **Transcript ingestion is heuristic-only by design** (no hard LLM dependency) so it can run as a background task without provider credentials.
- The `profile_md_renderer` deliberately does **not** touch the `connected-accounts` block — that's owned by the Composio provider path.
