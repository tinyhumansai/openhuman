# Migrating `src/openhuman/inference/` onto `vendor/tinyagents/`

**Status:** plan (not started)
**Relates to:** #4249 (tinyagents migration), `docs/tinyagents-migration-spec.md` (Phase 2 — model registry / profiles / fallback), `docs/tinyagents-full-migration-plan/`, `docs/tinyagents-drift-ledger.md`
**tinyagents:** 1.7.1, vendored as a git submodule at `vendor/tinyagents/` and path-patched over the crates.io pin (root `Cargo.toml`: `tinyagents = { version = "1.7", features = ["sqlite", "repl"] }` + `[patch.crates-io] tinyagents = { path = "vendor/tinyagents" }`).

---

## 1. Why

The agent loop already runs on tinyagents; `run_turn_via_tinyagents_shared` drives every turn. But the **model layer underneath it is still entirely in-house**: the harness reaches a crate `ChatModel` only through the `ProviderModel` adapter (`src/openhuman/tinyagents/model.rs`), which wraps openhuman's own `Box<dyn Provider>` stack from `src/openhuman/inference/provider/` — ~29.6k lines that re-implement what tinyagents 1.7 now ships natively:

| openhuman (`inference/provider/`, etc.)                                  | tinyagents 1.7 equivalent                                                                              |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| `Provider` trait, `ChatMessage`/`ChatRequest`/`ChatResponse`/`ProviderDelta` (`traits.rs`) | `harness::model::{ChatModel, ModelRequest, ModelResponse, ModelStream, ModelStreamItem}` + `harness::message::*` |
| `compatible*.rs` (~15 files: OpenAI-compat client, SSE streaming, parse, dump, repeat, timeout) | `harness::providers::openai` (convert / sse / transport / types) — serves every OpenAI-compatible endpoint |
| `reliable.rs` retry/backoff wrapper                                       | `harness::retry::{RetryPolicy, FallbackPolicy, RateLimiter, is_retryable}`                             |
| `router.rs` `RouterProvider` hint-table multi-model routing               | `harness::model::ModelRegistry` (per-depth model resolution)                                            |
| `model_context.rs` context-window table                                   | `MODEL_CONTEXT_PATTERNS` fallback + `registry::ModelCatalog` (offline snapshot: windows, pricing, capability flags) |
| `error_classify.rs` / `error_code.rs` (retryability, HTTP status parsing)  | `harness::retry::is_retryable` + `harness::model::ProviderError`                                        |
| provider-string → provider construction (`factory.rs`, partially)          | `harness::providers::{ProviderKind, ProviderSpec}` factory (`ProviderKind::infer`, `Compatible`)        |
| embeddings dispatch (`ops.rs` → local/cloud)                               | `harness::embeddings` traits (+ openai impl)                                                            |
| `provider/openai_codex.rs`, `openhuman_backend.rs`, `claude_agent_sdk/`    | no equivalent — stay as host `ChatModel` impls                                                          |

Maintaining both stacks means every fix (streaming edge case, retry policy, context-window entry, provider quirk) lands twice, and the adapter seam (`ProviderModel` + `ThinkingForwarder` + usage-carry plumbing) exists only to translate between two isomorphic type systems. Since `vendor/tinyagents` is our own crate (same GPL-3.0 license, same org), host-agnostic gaps get **upstreamed into the crate**; only genuinely openhuman-specific glue stays.

**End state:** the crate's `ChatModel`/`ModelRequest` is the native model interface everywhere in openhuman; `inference/` keeps only host concerns (RPC surface, config, local runtime management, voice, OAuth, `/v1` endpoint); `ProviderModel` and the entire `Provider` trait stack are deleted.

### Blast radius

- 170 files outside `inference/` import `openhuman::inference`; 151 import `inference::provider` specifically. Top consumers: agent harness/session/tools/triage, `context`, `voice`, `routing`, `memory_tree`, `learning`, `channels`, `embeddings`, `subconscious`, `screen_intelligence`, `threads`, `migrations`, `config/schema`.
- Module sizes: `provider/` ≈ 29.6k lines, `local/` ≈ 13.7k, `voice/` + `http/` + `openai_oauth/` ≈ 4.4k, root files ≈ 5.5k. Only `provider/` + parts of the root files migrate; the rest stays.

---

## 2. Disposition map

### Migrates (replaced by crate, or upstreamed into crate)

| Component | Destination | Notes |
| --- | --- | --- |
| `provider/traits.rs` — `Provider` trait + request/response/delta types | **delete**, use crate `ChatModel` + message types | The big inversion (Phase 1). `UsageInfo` → crate `Usage` (see gap G1 on USD/cached tokens). |
| `provider/compatible*.rs` — OpenAI-compat client | **delete**, use crate `providers::openai` | Gap audit first (Phase 2): request-dump debugging, repeat-detection, per-request timeout, BYOK auth styles. |
| `provider/reliable.rs` | **delete**, use crate `RetryPolicy` (+ `FallbackPolicy`) | Also resolves the known double-retry (reliable.rs *and* harness-level retry both fire today). |
| `provider/router.rs` (`RouterProvider`) | **delete**, use crate `ModelRegistry` | Hint table (`reasoning-v1`, `agentic-v1`, …) becomes registry aliases. |
| `model_context.rs` (`context_window_for_model`) | **upstream** entries into crate `ModelCatalog` snapshot / `MODEL_CONTEXT_PATTERNS`; host keeps a thin lookup that prefers config overrides | Crate catalog also carries pricing + capability flags — feeds the cost tracker. |
| `provider/error_classify.rs`, `error_code.rs` | **delete**, use crate `is_retryable` / `ProviderError` | Upstream any status-classification the crate misses. |
| `provider/temperature.rs` (`@<temp>` suffix) | host parses the suffix in the factory; value rides `ModelRequest` params | Grammar stays host-side; the plumbing type goes away. |
| `provider/config_rejection.rs`, `billing_error.rs` | **split**: generic classification upstreamed as crate error kinds; openhuman semantics (Sentry demotion, budget messaging) stay as a host classifier over `TinyAgentsError` | |
| `provider/factory.rs` | **shrinks, stays**: resolves openhuman provider strings (`openhuman`, `cloud`, `ollama:<model>`, `<slug>:<model>[@temp]`) + config + credentials → crate `ProviderSpec`/`Arc<dyn ChatModel>` | This is the host↔crate boundary after migration. `BYOK_INCOMPLETE_SENTINEL` stays. |
| `provider/ops.rs` (`list_configured_models`, SessionExpired publishing) | **stays**, retargeted to crate types | SessionExpired needs an auth-failure signal from the crate client (gap G3). |
| embeddings dispatch | crate `harness::embeddings` traits (seam `tinyagents/embeddings.rs` already exists — finish it) | Local (Ollama) embedding stays a host impl of the crate trait. |
| `provider/thread_context.rs`, `resolved_route.rs`, `auth_error_registry.rs` | **re-home** into `src/openhuman/tinyagents/` (they're seam concerns, not provider concerns) | `thread_context` task-locals already consumed by `model.rs`. |

### Stays in `inference/` (host concerns, out of scope for the crate)

- **RPC surface**: `schemas.rs`, `ops.rs`, `local/schemas.rs` — all `inference.*` controllers, legacy aliases.
- **Local runtime management** (`local/`): Ollama/LM Studio detect/spawn/adopt, Whisper/Piper install, download progress, model artifacts, context floor. The *chat client* to Ollama/LM Studio migrates to crate `ProviderKind::Ollama`/`Compatible`; process lifecycle stays.
- **`voice/`**: STT/TTS inference impls (whisper-cpp bindings, Piper) — not LLM-shaped; unchanged.
- **`openai_oauth/`**: Codex OAuth PKCE + encrypted token store (credentials domain integration).
- **`http/`**: the `/v1/chat/completions` OpenAI-compat *server* endpoint. (Later option: crate 1.3+ has OpenAI-compat runtime model listing; revisit after Phase 5.)
- **`device.rs`, `presets.rs`, `model_ids.rs`, `paths.rs`, `parse.rs`**: hardware profiles, preset tiers, config-derived model-id resolution, artifact paths.
- **`sentiment.rs`**: stays as a host op, but its model call is rewritten onto `ChatModel` (+ crate structured output) in Phase 6.
- **Bespoke providers** (`openhuman_backend.rs` session-JWT managed backend, `claude_agent_sdk/` subprocess, `openai_codex.rs` OAuth-token compat variant): stay in-repo, reimplemented as crate `ChatModel` impls (Phase 4).

---

## 3. Known crate gaps to close first (upstream work in `vendor/tinyagents`)

Verified against 1.7.1 source; re-audit at Phase 0 since the crate moves fast.

- **G1 — Usage fidelity**: crate `Usage` has no `charged_amount_usd` and needs verifying for cache-read/cache-write token fields. The $0-cost-turn bug (fixed host-side via `cost::catalog::estimate_cost_usd`) shows exactly what breaks when this is lossy. Either upstream optional cost/cached fields on `Usage`, or keep host-side estimation keyed off the crate `ModelCatalog` pricing.
- **G2 — Tool-call start metadata**: crate `ToolDelta` carries `call_id`/`content` but no `tool_name`; the UI timeline's tool-start event is still forwarded out-of-band (`model.rs` forwarder). Upstream `tool_name` on the first `ToolDelta` (or a dedicated start item) so the forwarder can die with `ProviderModel`.
- **G3 — Auth-failure signal**: openhuman publishes `DomainEvent::SessionExpired` when a chat attempt fails auth. The crate client must classify 401/expired distinctly (as a `ProviderError` kind) so the host factory can hook it without string-sniffing.
- **G4 — Request dump / wire observability**: `compatible_dump.rs` writes raw request/response dumps for debugging. Upstream a transport-level hook (or confirm the crate's observability exporters cover it) before deleting.
- **G5 — Per-request timeout policy**: `compatible_timeout.rs` semantics vs. what the crate transport exposes. Upstream a per-call timeout on `ModelRequest`/`ProviderSpec` if missing.
- **G6 — BYOK auth styles**: the provider preset catalog (`docs/inference-provider-catalog.md`) supports multiple `AuthStyle`s (headers etc.). Confirm crate `ProviderSpec` can express every style in the catalog; upstream what's missing.
- **G7 — Repeat-output guard**: `compatible_repeat.rs` (degenerate-repetition detection). Decide: upstream as an optional stream guard, or accept the loss (note #4463 already tracks deleted repeat guards).

Upstream flow: change in `vendor/tinyagents` (submodule working tree) → PR to `tinyhumansai/tinyagents` → publish → bump the crates.io pin in **both** Cargo worlds (root + `app/src-tauri`) and the submodule ref in lockstep. Nothing in openhuman may depend on unpublished vendored-only API at a merge point.

---

## 4. Phases

Each phase compiles green in both Cargo worlds, keeps ≥80% diff coverage, and lands as its own PR-sized slice. Provider-string grammar, RPC names, and observable UI behavior (streaming, cost footer, tool timeline) are parity-locked throughout.

### Phase 0 — Inventory & gap re-audit
- Enumerate every consumer of `Provider` / `ChatRequest` / `ChatResponse` / `ChatMessage` outside `inference/` (151 files) and bucket them: (a) goes through the seam already, (b) direct one-shot `provider.chat(...)` callers (learning, memory, subconscious, screen_intelligence, sentiment, triage…), (c) type-only imports.
- Re-verify §3 gaps against current crate HEAD; file crate issues; update `docs/tinyagents-sdk-gaps.md`.
- Golden-transcript capture: record request/response wire dumps for the BYOK catalog matrix + Ollama + openhuman backend on the current stack, as fixtures for Phase 2 parity.

**Exit:** disposition table confirmed per-file; crate gap PRs filed.

### Phase 1 — Model-layer inversion (the pivot)
- Introduce `create_chat_model(...) -> Arc<dyn ChatModel>` in `factory.rs` alongside `create_chat_provider`, initially wrapping the existing stack via `ProviderModel` (zero behavior change).
- Migrate consumers bucket-by-bucket from `Box<dyn Provider>` to `Arc<dyn ChatModel>`: one-shot callers first (they use `chat()` once — mechanical: `ChatRequest` → `ModelRequest::new(...)`), then the seam (`run_turn_via_tinyagents_shared` takes the model directly — delete the wrap at the call sites), then streaming consumers.
- Keep a temporary reverse adapter (`ChatModel` → `Provider`) only if a consumer can't move in one slice; delete it before phase exit.

**Exit:** no caller outside `inference/provider/` names the `Provider` trait; `ProviderModel` is constructed in exactly one place (factory).

### Phase 2 — OpenAI-compatible client swap
- Behind the factory, construct crate `providers::openai` clients (via `ProviderSpec`) instead of `CompatibleProvider` for: BYOK slugs, Ollama, LM Studio, the `cloud` slug.
- Parity-test against Phase 0 golden dumps: request shape (tools JSON, temperature, multimodal blocks), SSE streaming (text, reasoning, tool-arg deltas), usage extraction, error mapping. Note recent regression surface: tool-calling defaults to JSON with P-Format opt-in (9b84f9684) — the crate path must honor the same default.
- Delete `compatible*.rs` (15 files, the bulk of the 29.6k lines) once all construction sites are switched.

**Exit:** no `compatible*.rs` left; wire parity fixtures green; `pnpm test:rust` + `json_rpc_e2e` green.

### Phase 3 — Reliability, routing, model metadata
- Replace `ReliableProvider` layering (`session/builder/factory.rs` re-layered it in the P1 parity fix) with crate `RetryPolicy` at the client level; audit and remove the double-retry.
- Replace `RouterProvider` with `ModelRegistry`: abstract tier names (`reasoning-v1`, `coding-v1`, …) become registry entries resolved per call; `provider_for_role`/workload resolution feeds the registry instead of building a router provider.
- Upstream openhuman's context-window table entries into the crate `ModelCatalog` snapshot; `context_window_for_model` becomes a host shim: config override → catalog → crate pattern fallback. Wire catalog pricing into `cost::catalog` (replacing the hand-rolled rate table, or seeding it).

**Exit:** `reliable.rs`, `router.rs`, `model_context.rs` deleted; retry fires exactly once per layer by design.

### Phase 4 — Bespoke providers as `ChatModel` impls
- Rewrite `openhuman_backend.rs` (managed backend, session JWT + SessionExpired publishing via G3), `openai_codex.rs` (Codex OAuth token source over the crate openai client), and `claude_agent_sdk/` (subprocess protocol) as direct `ChatModel` implementations in their current homes.
- Local runtime: `local/` keeps process lifecycle; its chat/vision/embed entrypoints call crate clients pointed at the local base URL.

**Exit:** `Provider` trait has zero implementations → delete `traits.rs` and the trait itself.

### Phase 5 — Seam shrink
- Delete `ProviderModel`, `ThinkingForwarder` remnants, `ProviderUsageCarry`, and the `ChatMessage`↔crate-message conversion layer in `tinyagents/convert.rs` (the harness now receives crate types natively).
- Re-home `thread_context.rs` / `resolved_route.rs` / `auth_error_registry.rs` into `src/openhuman/tinyagents/`.
- `inference/provider/` collapses to: `factory.rs` (string grammar → `ChatModel`), bespoke impls, host error classifier, `ops.rs`, `schemas.rs`.

**Exit:** `src/openhuman/tinyagents/model.rs` deleted; adapter inventory test updated.

### Phase 6 — One-shot inference ops onto the crate
- `sentiment.rs`, `should_react`, `summarize`, vision prompts, triage-style single calls: rewrite onto `ChatModel` + crate structured output (`harness/structured`) instead of hand-rolled parse (`parse.rs` shrinks or dies).
- Embeddings: finish `tinyagents/embeddings.rs` — cloud via crate openai embeddings, local as a host impl of the crate trait; `LocalAiEmbeddingResult` maps from crate types.

**Exit:** no ad-hoc prompt/parse loops outside the crate surface.

### Phase 7 — Cleanup, docs, deletion ledger
- Update `inference/README.md`, `gitbooks/developing/architecture/agent-harness.md`, `docs/inference-provider-catalog.md`; add deletions to `docs/tinyagents-full-migration-plan/99-deletion-ledger.md`; refresh `docs/tinyagents-drift-ledger.md`.
- Remove dead re-exports from `inference/mod.rs`; keep temporary `pub use` shims only where a follow-up PR is already open.
- Sweep for stale doc-comments naming `Provider`/`CompatibleProvider`.

---

## 5. Risks & gotchas

- **Wire-format regressions are silent until a provider hiccups.** The compat client encodes years of quirk handling (SSE edge cases, malformed tool-arg fragments, providers that omit usage). Mitigation: Phase 0 golden dumps + per-provider live smoke tests gated on env keys, run against the real BYOK matrix before each deletion.
- **Cost accounting**: the event bridge + `record_unobserved_turn_usage` fallback were hard-won ($0-turn bug). Any `Usage` shape change must keep cached-token and USD flow intact end-to-end (dashboard + footer).
- **Streaming UI parity**: tool-start events (G2) and post-hoc reasoning still ride the out-of-band forwarder; deleting it before the crate gap closes breaks the tool timeline.
- **Two Cargo worlds**: root and `app/src-tauri` pin tinyagents independently — every crate bump lands in both lockfiles plus the submodule ref, same commit.
- **Vendored-crate discipline**: the path patch means local vendor edits silently take effect; CI and other clones need the submodule at the matching ref. Never merge openhuman code that requires unpublished crate API.
- **Sentry noise contract**: `ops.rs` deliberately demotes provider/user-config failures to `warn!`. The new host classifier over `TinyAgentsError` must preserve `expected_error_kind` behavior or Sentry floods.
- **Test serialization**: everything runs under `inference_test_guard()` (process-global mutex over the runtime singleton + config); new tests must too.
- **Open regressions in the same area** (#4451–#4469, esp. #4460 streamed calls losing thread_id task-locals, #4463 repeat guards): coordinate so this migration doesn't re-break or mask those fixes; thread-context task-locals move in Phase 5 — verify #4460's fix survives the re-home.
- **`/v1` server endpoint** reuses provider types for its request/response DTOs (`http/types.rs`) — it must keep its external wire shape while internals switch to crate types.

## 6. Explicit non-goals

- No change to the `inference.*` RPC surface, provider-string grammar, or the Settings > AI preset catalog UX.
- No migration of `local/` process management, `voice/` STT/TTS engines, `openai_oauth/` flows, or `device`/`presets`/`paths` — they are not LLM-driven in the crate's sense.
- No sub-agent execution changes (P5 of the harness migration already declined crate `SubAgentTool`).
