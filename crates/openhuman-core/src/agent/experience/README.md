# agent_experience

Hermes-style **procedural experience memory** for agents. Captures what tool sequences worked (or failed) during a chat turn, redacts secrets, persists them as structured records in the memory store, and ranks/injects relevant past experiences back into future turns as a compact "Relevant Operating Experience" prompt block. The goal is cross-turn procedural learning: the agent remembers *how* it solved similar tasks before, not just facts.

## Responsibilities

- Persist experiences (upsert by stable id) into the memory store under the `agent_experience` namespace, scrubbing free-text fields first.
- Mark experiences as dismissed so retrieval skips them.
- Auto-derive experience candidates from a completed turn's tool calls (multi-tool success, repeated failures, partial recovery) via a `PostTurnHook`.
- Render ranked hits into a byte-capped markdown block and prepend it to the enriched user message before a turn.
- Expose capture/retrieve/list/dismiss over JSON-RPC.
- Provide `DriverMemory`, the `Memory`-trait view of the bound memory driver that the store (and the session builder) run on.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/agent/experience/mod.rs` | Export-focused module root; re-exports the public surface. |
| `crates/openhuman-core/src/agent/experience/prompt.rs` | `render_experience_hits` (byte-capped markdown under `AGENT_EXPERIENCE_HEADING = "## Relevant Operating Experience"`) and `prepend_experience_block`. |
| `crates/openhuman-core/src/agent/experience/schemas.rs` | Controller schemas + `handle_*` dispatchers; `all_controller_schemas` / `all_registered_controllers`. |

## Public surface

From `mod.rs` re-exports:

- `AgentExperienceCaptureHook` (capture)
- `prepend_experience_block`, `render_experience_hits`, `AGENT_EXPERIENCE_HEADING` (prompt)
- `all_agent_experience_controller_schemas`, `all_agent_experience_registered_controllers` (schemas)
- `retrieve_across_stores`, `AgentExperienceStore`, `ExperienceQuery`, `AGENT_EXPERIENCE_NAMESPACE` (store)
- `redact_text`, `stable_experience_id`, `AgentExperience`, `ExperienceHit`, `ExperienceOutcome`, `ExperienceSource` (types)

`ops::DriverMemory` is `pub` but reached by path (`crate::agent::experience::ops::DriverMemory`), not re-exported.

## RPC / controllers

Namespace `agent_experience` (registered into `crates/openhuman-core/src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |


## Agent hooks (not a tool)


- **Multi-tool success**: ≥2 successful tool calls → `ExperienceOutcome::Success`, confidence 0.72.
- **Repeated failure**: a tool that failed ≥2 times in one turn → `Failure`, confidence 0.68, with an error class parsed from the output summary (`...(error_class)`).
- **Partial success**: a failure followed by a later success (≥2 calls total) → `Partial`, confidence 0.62; skipped when it would duplicate an earlier candidate's id or outcome.

## Events

None — no `bus.rs`; this module does not publish or subscribe to `DomainEvent`s.

## Persistence

Records are stored through the `Memory` trait (no dedicated DB), served by `DriverMemory` over the memory driver bound for the workspace subtree:

- Namespace: `agent_experience` (`AGENT_EXPERIENCE_NAMESPACE`).
- Value: **base64 of the `AgentExperience` JSON**, `MemoryCategory::Custom("agent_experience")`. Base64 keeps the memory layer's bare-numeric PII scrubber from rewriting a Luhn-valid millisecond timestamp and corrupting the JSON (#5209); reads fall back to plain JSON for legacy rows.

## Dependencies

- `crate::memory` — `Memory` trait, `MemoryCategory`, `memory::binding::{for_config, for_subtree}` (driver binding behind `DriverMemory`), `memory::api::{provider, recall, types, health}` (the provider contract `DriverMemory` adapts), `memory::safety::sanitize_text` (store-time scrub), `memory::source_scope::as_bus_scope` (explicit recall scope), `memory::preferences::recall_by_vector_over`.
- `crate::config` — `Config::load_or_init` for `workspace_dir` and `subsystems.memory` when the RPC handlers bind a store.
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for RPC registration.
- `crate::core` — `ControllerSchema`, `FieldSchema`, `TypeSchema` (schema types); `crate::rpc::RpcOutcome`.
- `crate::memory::tool_memory::test_helpers::MockMemory` and `crate::memory::guard::test_support::RecordingProvider` — tests only.

## Used by

- `crates/openhuman-core/src/core/all.rs` — registers controllers/schemas and the namespace description.
- `crates/openhuman-core/src/agent/mod.rs` — declares `pub mod experience`.
- `crates/openhuman-core/src/agent/session_host/turn/core/experience_context.rs` — defines `Agent::inject_agent_experience_context`, which queries the session store plus the shared store with `retrieve_across_stores` (max 3 hits, 2048-byte block, gated on `learning_enabled`) and prepends the block to the enriched user message. Called from `turn/core_turn.rs`.
- `crates/openhuman-core/src/agent/tinyagents/host/experience_store.rs` — host adapter implementing `tinyagents_harness::host::ExperienceStore` over `AgentExperienceStore`.
- `crates/openhuman-core/src/config/migration_helpers/core.rs` — uses `DriverMemory::for_config` to bind the import target.

## Notes / gotchas

- **Two redaction layers at write time**: `capture::build_experience` masks `Bearer …`, `sk-…`, and `token=/password:` pairs with `types::redact_text`; `store::put` then runs the full `memory::safety::sanitize_text` scrubber (private keys, vendor secrets, national-ID / phone / card PII) over the free-text fields. The base64 payload means the memory layer's own content scrub is a no-op, so the store-level scrub is what preserves the invariant.
- Retrieval scoring is **lexical, not embedding-based**: term sets keep only tokens length > 2, normalized lowercase; score combines tool overlap (weighted highest), tag overlap, query-term overlap over summary+lesson+hints, plus small agent/entrypoint match boosts and a confidence prior. `max_hits == 0` short-circuits to empty. The live-turn path additionally drops hits with no `match_reasons`.
- `render_experience_hits` is hard byte-capped (`max_bytes`) with UTF-8-boundary-safe truncation, so the injected prompt block can't blow the context budget.
- The capture hook is gated by an `enabled` flag passed at construction; when disabled `on_turn_complete` is a no-op, and capture failures only `log::warn!` (never fail the turn).
- `DriverMemory` wraps the driver, not `MemoryGuard`, on purpose: the guard truncates `store` content at `capture_max_chars`, and truncated base64 does not decode. Its `ops.rs` rustdoc notes it belongs next to `memory::binding` and only sits here because both callers do.
