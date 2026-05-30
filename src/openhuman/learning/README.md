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
| `candidate.rs` | Phase 1 taxonomy: `FacetClass`, `CueFamily` (+ `weight()`), `EvidenceRef`, `LearningCandidate`, and the bounded FIFO `Buffer` with a `global()` singleton (cap 1024). |
| `cache.rs` | `FacetCache` — typed wrapper over `user_profile_facets`; class↔key helpers (`class_from_key`, `key_with_class`, `class_prefix`). Delegates to `memory_store::profile`. |
| `stability_detector.rs` | Phase 3 `StabilityDetector::rebuild` — the scoring/budget/state-assignment cycle; thresholds, half-lives, budgets, and the `stability()` formula. Publishes `CacheRebuilt`. |
| `scheduler.rs` | Periodic rebuild loop (`spawn_rebuild_loop`, default 30 min) + event-driven debounced trigger (`register_event_trigger`) subscribing to memory/tree-summarizer events. |
| `schemas.rs` | All RPC controller schemas + `handle_*` async handlers for the `learning.*` namespace. |
| `reflection.rs` | `ReflectionHook` post-turn hook: heuristic reflection-cue capture + LLM reflection, stores observations/patterns/preferences/reflections, emits Goal/Style candidates. |
| `tool_tracker.rs` | `ToolTrackerHook` post-turn hook + `ToolStats`; per-tool running success/failure/duration tallies in the `tool_effectiveness` memory category. |
| `user_profile.rs` | `UserProfileHook` post-turn hook; Aho-Corasick DFA over curated preference phrases, stores matches in the `user_profile` category. |
| `prompt_sections.rs` | `LearnedContextSection`, `UserProfileSection`, `MemoryAccessSection` (+ `MEMORY_ACCESS_INSTRUCTION`), and `load_learned_from_cache` (synchronous cache→prompt loader, cap 25). |
| `profile_md_renderer.rs` | `ProfileMdRenderer` — subscribes to `CacheRebuilt`, re-renders 5 managed `PROFILE.md` blocks (style/identity/tooling/vetoes/goals) from Active facets. |
| `linkedin_enrichment.rs` | Gmail→LinkedIn→Apify enrichment pipeline; `run_linkedin_enrichment`, `summarise_profile_with_llm`, `render_profile_markdown`, `scrape_linkedin_profile`. |
| `extract/` | Phase 2 producers (`mod.rs` + `signature.rs`, `heuristics.rs`, `summary_facets.rs`). |
| `extract/signature.rs` | Email-signature parser → Identity candidates; subscribes to `DocumentCanonicalized` (email). |
| `extract/heuristics.rs` | `LengthRatioDetector` / `EditWindowDetector` / `CorrectionRepeatDetector` → Style + Veto candidates via `record_turn`. |
| `extract/summary_facets.rs` | Parses the LLM summariser's structured JSON block; `route_facets_to_buffer` pushes validated candidates (requires `evidence_chunks`). |
| `transcript_ingest/` | Transcript→memory pipeline (`mod.rs`, `extract.rs`, `dedupe.rs`, `persist.rs`, `types.rs`). |
| `transcript_ingest/mod.rs` | `ingest_transcript_path` / `ingest_session_transcript`; extract→dedupe→persist into `conversation_memory` + `conversation_reflections` namespaces. |
| `*_tests.rs` | Sibling test suites (`cache`, `reflection`, `prompt_sections`, `linkedin_enrichment`). |

## Public surface

Re-exported from `mod.rs`:

- **Candidate types**: `Buffer`, `CueFamily`, `EvidenceRef`, `FacetClass`, `LearningCandidate`.
- **Cache / detector**: `FacetCache`, `StabilityDetector`.
- **Hooks**: `ReflectionHook`, `ToolTrackerHook`, `UserProfileHook` (all impl `PostTurnHook`).
- **Prompt**: `LearnedContextSection`, `UserProfileSection`, `MemoryAccessSection`, `MEMORY_ACCESS_INSTRUCTION`, `load_learned_from_cache`.
- **Profile**: `ProfileMdRenderer`.
- **Schemas**: `all_learning_controller_schemas`, `all_learning_registered_controllers`, `learning_schemas`.

Not re-exported but public within the module: `candidate::global()`, the `linkedin_enrichment::*` pipeline fns, `transcript_ingest::ingest_*`, `scheduler::{spawn_rebuild_loop, register_event_trigger, DEFAULT_REBUILD_INTERVAL}`, and `stability_detector` constants (`TAU_*`, `HALF_LIFE_*`, `BUDGET_*`, `stability`, `half_life`, `class_budget`).

## RPC / controllers

Namespace `learning` (wired into `src/core/all.rs`; 11 controllers). Methods:

| Method | Purpose |
| --- | --- |
| `learning.linkedin_enrichment` | Run the Gmail→LinkedIn→Apify pipeline (optional `profile_url` to skip Gmail search). |
| `learning.save_profile` | Write markdown to `{workspace_dir}/PROFILE.md`; optional `summarize` runs it through the LLM compressor first. |
| `learning.rebuild_cache` | Manually trigger a `StabilityDetector` rebuild; returns added/evicted/kept/total_size. |
| `learning.cache_stats` | Cache totals + per-state and per-class breakdown. |
| `learning.list_facets` | List Active + Provisional facets, optional `class` filter. |
| `learning.get_facet` | Fetch one facet by `class` + `key` suffix. |
| `learning.update_facet` | Set a facet value and pin it (`user_state = Pinned`). |
| `learning.pin_facet` / `learning.unpin_facet` | Toggle `user_state` Pinned ↔ Auto. |
| `learning.forget_facet` | Mark `Dropped` + `user_state = Forgotten` (blocks re-promotion). |
| `learning.reset_cache` | Delete all `Auto` rows, preserve `Pinned`. |

All handlers go through the memory client's `profile_conn()` and a `FacetCache`; `linkedin_enrichment` / `save_profile` load config via `config::rpc::load_config_with_timeout`.

## Agent tools

The module defines no `tools.rs`. However the `tool_effectiveness` stats it writes are surfaced by the cross-cutting `tool_stats` tool in `src/openhuman/tools/impl/system/tool_stats.rs`, and `memory_tools` references learning namespaces.

## Events

Uses the typed event bus (`src/core/event_bus/`):

- **Publishes**: `DomainEvent::CacheRebuilt { added, evicted, kept, total_size, rebuilt_at }` after each rebuild (`stability_detector.rs`).
- **Subscribes**:
  - `profile_md_renderer.rs` (`ProfileMdRenderer::subscribe`) → `CacheRebuilt` → re-render `PROFILE.md` blocks.
  - `scheduler.rs` (`RebuildTriggerHandler`, domains `memory` / `tree_summarizer`) → `DocumentCanonicalized` (email/document) and `TreeSummarizerPropagated` → debounced (60 s) rebuild.
  - `extract/signature.rs` → `DocumentCanonicalized` (email) → emit Identity candidates.

These are subscriber registrations rather than a single `bus.rs`; subscriptions must keep their `SubscriptionHandle` alive (callers store them in statics).

## Persistence

- **`user_profile_facets`** (the ambient cache) — accessed via `FacetCache`, backed by `memory_store::profile` helpers (`profile_select_active/all`, `profile_get_by_key`, `profile_upsert_full`, `profile_set_user_state`, `profile_delete_*`). Stores `ProfileFacet { key, value, state, user_state, stability, confidence, evidence_count, evidence_refs, class, cue_families, first/last_seen_at, … }`.
- **KV memory namespaces** (via the `Memory` trait): `learning_observations`, `learning_patterns`, `learning_reflections`, `user_profile`, `tool_effectiveness`, plus transcript-ingest `conversation_memory` / `conversation_reflections`. LinkedIn enrichment also persists via `MemoryClient::store_skill_sync` and writes `{workspace_dir}/PROFILE.md`.
- **In-memory**: the global `candidate::Buffer` (transient evidence, not persisted) and per-session state in `extract/heuristics.rs`.

## Dependencies

- `crate::openhuman::memory_store::profile` — the `ProfileFacet` / `FacetState` / `UserState` types and the SQL helpers backing `FacetCache` (heaviest dependency).
- `crate::openhuman::memory` / `memory_store` — the `Memory` trait, `MemoryClient`, categories; all KV persistence and the global memory client used by RPC handlers.
- `crate::openhuman::agent::hooks` — `PostTurnHook` / `TurnContext` / `ToolCallRecord` implemented by the three hooks.
- `crate::openhuman::agent::harness::session::transcript` — `SessionTranscript` parsing for transcript ingestion.
- `crate::openhuman::inference::provider` / `inference::local` — LLM calls for reflection and profile summarisation.
- `crate::openhuman::config` — `Config` / `LearningConfig` / `ReflectionSource` feature flags and `config::rpc` loader.
- `crate::openhuman::context::prompt` — `PromptContext` / `PromptSection` / `LearnedContextData` for prompt injection.
- `crate::openhuman::composio` — `composio::client` (Gmail fetch for enrichment) and `composio::providers::profile_md` (`replace_managed_block` for `PROFILE.md`).
- `crate::openhuman::integrations` — `build_client` / `IntegrationClient` for the Apify scrape call.
- `crate::core::event_bus` — publish/subscribe + `EventHandler` for `CacheRebuilt` and trigger handlers.
- `crate::core::all` / `crate::rpc` — controller registry types (`RegisteredController`, `ControllerFuture`) and `RpcOutcome`.

## Used by

- `src/core/all.rs` — registers the `learning.*` controllers + schemas.
- `src/openhuman/agent/harness/session/{builder,turn}.rs` and `agent/memory_loader.rs` — wire the post-turn hooks, prompt sections, and learned-context loading into the agent loop.
- `src/openhuman/channels/runtime/startup.rs` — likely registers schedulers/subscribers at startup.
- `src/openhuman/memory_store/unified/profile.rs`, `memory_sync/composio/providers/profile.rs`, `memory_tools/{capture,mod}.rs`, `tools/impl/system/tool_stats.rs`, `tools/schemas.rs` — consume facet/learning types.

## Notes / gotchas

- **Pinned ⇒ stability ∞, Forgotten ⇒ stability 0.** User overrides hard-win over scoring in both `stability()` and `state_from_stability()`. `update_facet` implicitly pins.
- **Class is encoded in the key prefix** (`style/verbosity`, `goal/learn_rust`). `candidate.key` carries no prefix; the detector prepends `class_prefix`. Legacy rows without a recognised prefix are skipped by rebuild.
- **`emit_candidates_*` uses the global buffer length as a synthetic `episodic_id`** (a Phase-2 placeholder noted to be replaced by a real `episodic_log` row id).
- **Goal facets render value-only** (full sentence, no key prefix) in both prompt injection and `PROFILE.md`; other classes render `**key**: value`.
- **Prompt loaders are synchronous SQLite reads** — `load_learned_from_cache` runs in the sync prompt-build path and degrades to empty on error. Both the cache path and the legacy KV-namespace path are still active (KV slated for later removal).
- **Reflection has a local/cloud gate.** Local route requires `local_ai.usage.learning` flag; if off it falls back to a cloud provider or no-ops (empty string), and an empty response is a clean-skip sentinel that rolls back the throttle counter. Per-session reflections are throttled by `max_reflections_per_session`.
- **LinkedIn enrichment short-circuits** if `PROFILE.md` already exists, and the Composio-only Gmail-search stage is documented as currently erroring (Gmail-via-Composio removed) — callers should pass `preset_profile_url` obtained via the frontend's webview Gmail helper.
- **Transcript ingestion is heuristic-only by design** (no hard LLM dependency) so it can run as a background task without provider credentials.
- The `profile_md_renderer` deliberately does **not** touch the `connected-accounts` block — that's owned by the Composio provider path.
