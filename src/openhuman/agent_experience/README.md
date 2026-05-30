# agent_experience

Hermes-style **procedural experience memory** for agents. Captures what tool sequences worked (or failed) during a chat turn, redacts secrets, persists them as structured records in the shared memory store, and ranks/injects relevant past experiences back into future turns as a compact "Relevant Operating Experience" prompt block. The goal is cross-turn procedural learning: the agent remembers *how* it solved similar tasks before, not just facts.

## Responsibilities

- Define the `AgentExperience` record (task summary, tool sequence, outcome, lesson, reuse/avoid hints, confidence, tags).
- Persist experiences (upsert by stable id) into the memory store under the `agent_experience` namespace, redacting secret-like text first.
- Retrieve and rank experiences for a given task query via lexical/tool/tag overlap scoring.
- Mark experiences as dismissed so retrieval skips them.
- Auto-derive experience candidates from a completed turn's tool calls (multi-tool success, repeated failures, partial recovery) via a `PostTurnHook`.
- Render ranked hits into a byte-capped markdown block and prepend it to the enriched user message before a turn.
- Expose capture/retrieve/list/dismiss over JSON-RPC.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/agent_experience/mod.rs` | Export-focused module root; re-exports the public surface. |
| `src/openhuman/agent_experience/types.rs` | Serde types (`AgentExperience`, `ExperienceHit`, `ExperienceSource`, `ExperienceOutcome`), `redact_text` (Bearer / `sk-` / `token=secret` masking), and `stable_experience_id` (SHA-256 over summary + tool sequence + outcome). |
| `src/openhuman/agent_experience/store.rs` | `AgentExperienceStore` over `Arc<dyn Memory>`: `put`/`list`/`dismiss`/`retrieve`, `ExperienceQuery`, the `AGENT_EXPERIENCE_NAMESPACE` const, and the lexical/tool/tag overlap scoring (`score_experience`). |
| `src/openhuman/agent_experience/capture.rs` | `AgentExperienceCaptureHook` — a `PostTurnHook` that mines `TurnContext.tool_calls` into experience candidates (`successful_multi_tool_experience`, `repeated_failure_experiences`, `partial_success_experience`) and persists them. |
| `src/openhuman/agent_experience/prompt.rs` | `render_experience_hits` (byte-capped markdown under `AGENT_EXPERIENCE_HEADING = "## Relevant Operating Experience"`) and `prepend_experience_block`. |
| `src/openhuman/agent_experience/ops.rs` | RPC entry points returning `RpcOutcome<T>` (`capture`/`retrieve`/`list`/`dismiss`); `open_store()` resolves the memory client (lazy-init from config if not ready). |
| `src/openhuman/agent_experience/schemas.rs` | Controller schemas + `handle_*` dispatchers; `all_controller_schemas` / `all_registered_controllers`. |

## Public surface

From `mod.rs` re-exports:

- `AgentExperienceCaptureHook` (capture)
- `prepend_experience_block`, `render_experience_hits`, `AGENT_EXPERIENCE_HEADING` (prompt)
- `all_agent_experience_controller_schemas`, `all_agent_experience_registered_controllers` (schemas)
- `AgentExperienceStore`, `ExperienceQuery`, `AGENT_EXPERIENCE_NAMESPACE` (store)
- `redact_text`, `stable_experience_id`, `AgentExperience`, `ExperienceHit`, `ExperienceOutcome`, `ExperienceSource` (types)

## RPC / controllers

Namespace `agent_experience` (registered into `src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |
| `agent_experience.capture` | `experience: AgentExperience` | Stored `AgentExperience` (upserted, redacted). |
| `agent_experience.retrieve` | `query` (req), `tools[]`, `tags[]`, `agent_id?`, `entrypoint?`, `max_hits?` (default 5) | `hits: ExperienceHit[]` ranked. |
| `agent_experience.list` | none | `experiences: AgentExperience[]` ordered by most-recent update. |
| `agent_experience.dismiss` | `id` | `{ id, dismissed }`. |

All handlers delegate to `ops.rs` and wrap results in `RpcOutcome::single_log`.

## Agent hooks (not a tool)

This module owns no `tools.rs` agent tool. Instead it registers `AgentExperienceCaptureHook` as a **`PostTurnHook`** (`name() == "agent_experience_capture"`). On `on_turn_complete` it extracts candidates from the turn's tool calls and persists them when enabled. Candidate heuristics:

- **Multi-tool success**: ≥2 successful tool calls → `ExperienceOutcome::Success`, confidence 0.72.
- **Repeated failure**: a tool that failed ≥2 times in one turn → `Failure`, confidence 0.68, with an error class parsed from the output summary (`...(error_class)`).
- **Partial success**: a failure followed by a later success → `Partial`, confidence 0.62.

## Events

None — no `bus.rs`; this module does not publish or subscribe to `DomainEvent`s.

## Persistence

Records are stored through the shared `Memory` abstraction (no dedicated DB):

- Namespace: `agent_experience` (`AGENT_EXPERIENCE_NAMESPACE`).
- Key: `experience/<id>`; id is `stable_experience_id(...)` (`exp_<24 hex>`) when not supplied.
- Value: full `AgentExperience` JSON, `MemoryCategory::Custom("agent_experience")`.
- `put` preserves the original `created_at_ms` on update, stamps `updated_at_ms`, and redacts `task_summary` / `lesson` / `reuse_hint` / `avoid_hint` before write. Dismiss is a soft flag (`dismissed = true`), retained in `list`, filtered out of `retrieve`.

## Dependencies

- `crate::openhuman::memory` — `Memory` trait, `MemoryCategory`, and `memory::global` client (storage backend; lazy-init via `Config`).
- `crate::openhuman::config` — `Config::load_or_init` to resolve `workspace_dir` when the memory client isn't ready.
- `crate::openhuman::agent::hooks` — `PostTurnHook`, `TurnContext`, `ToolCallRecord` (capture hook contract / turn inputs).
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for RPC registration.
- `crate::core` — `ControllerSchema`, `FieldSchema`, `TypeSchema` (schema types); `crate::rpc::RpcOutcome`.
- `crate::openhuman::memory_tools::test_helpers::MockMemory` — tests only.

## Used by

- `src/core/all.rs` — registers controllers/schemas and the namespace description.
- `src/openhuman/agent/harness/session/builder.rs` — constructs `AgentExperienceCaptureHook::new(...)` and registers it for the learning/capture flow.
- `src/openhuman/agent/harness/session/turn.rs` — imports from this module and `inject_agent_experience_context` to retrieve + prepend the experience block into the enriched user message before a turn runs.
- `src/openhuman/mod.rs` — declares the module.
- `src/openhuman/memory_sync/workspace/mod.rs` — references it (doc comment) as a peer memory writer.

## Notes / gotchas

- **Redaction is applied at write time**, both in `store::put` and again in `capture::build_experience`; secret-like substrings (`Bearer …`, `sk-…`, `token=/password:` pairs) are masked before persistence.
- Retrieval scoring is **lexical, not embedding-based**: term sets keep only tokens length > 2, normalized lowercase; score combines tool overlap (weighted highest), tag overlap, query-term overlap over summary+lesson+hints, plus small agent/entrypoint match boosts and a confidence prior. `max_hits == 0` short-circuits to empty.
- `render_experience_hits` is hard byte-capped (`max_bytes`) with UTF-8-boundary-safe truncation, so the injected prompt block can't blow the context budget.
- The capture hook is gated by an `enabled` flag passed at construction; when disabled `on_turn_complete` is a no-op, and capture failures only `log::warn!` (never fail the turn).
