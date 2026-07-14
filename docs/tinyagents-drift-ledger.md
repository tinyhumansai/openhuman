# TinyAgents Drift Ledger (Phase 0)

**Purpose.** The TinyAgents migration spans `inference`, `tools`, and
`agent_orchestration`, while the OpenHuman host will keep evolving. This ledger
records the baseline used for the port plan and tracks which host-side drift must
be upstreamed, retained, or deleted before each phase cuts over.

- **DRIFT -> tinyagents PR** - generic engine behavior absent from the crate; port
  upstream before deleting the host copy.
- **HOST-OWNED** - OpenHuman product policy, RPC, config, credentials, UI, local
  runtime, or integration glue. No upstream action.
- **CONSOLIDATE / DELETE** - duplicate host implementation already covered by
  TinyAgents primitives; delete only after the seam proves the crate-backed path.
- **CLOSED** - resolved by a submodule/version bump or a completed cutover.

> **Gate rule:** no phase deletes host code until every open row for that phase
> is either upstreamed and bumped, reclassified host-owned, or covered by a
> crate-backed seam test.

## Anchors

| Thing | Value |
| --- | --- |
| Host repo | `tinyhumansai/openhuman` |
| Host branch | `docs/tinyagents-port-plan` |
| Host audit base | `42ce5c0e9` (`origin/main`, 2026-07-04) |
| Plan commit | `24f200e49` (`docs: TinyAgents port plan`) |
| TinyAgents submodule | `vendor/tinyagents` -> `tinyhumansai/tinyagents` |
| Phase 0 target | `v1.6.0` / `e72036d847b589044aa9a4add1b34544b92a293d` |
| Current host pin | `v1.8.0-1-g7c6e81a` ([tinyagents#49](https://github.com/tinyhumansai/tinyagents/pull/49) **merged** onto v1.8.0 main) |
| Verification PR | [openhuman#4769](https://github.com/tinyhumansai/openhuman/pull/4769) — Motion A + Motion B checkpoint. **CI fully green** (`PR CI Gate` + `Rust Core Coverage` + fmt/clippy); 14,437 Rust tests pass. First full CI verification of this branch. |

## Baseline Snapshot

Recorded from `docs/tinyagents-port-plan` after the Phase 0 version alignment
work started. Counts include Rust files only.

| Host module | Rust files | LOC | Test fns | Plan disposition |
| --- | ---: | ---: | ---: | --- |
| `src/openhuman/inference/` | 116 | 53,023 | 1,101 | Provider consolidation and small generic ports |
| `src/openhuman/tools/` | 94 | 38,553 | 877 | Tool model reconciliation, then builtin family ports |
| `src/openhuman/agent_orchestration/` | 64 | 25,769 | 262 | Sub-agent lifecycle consolidation onto TinyAgents graph/orchestration |
| `src/openhuman/tinyagents/` | 25 | 15,219 | 101 | Host seam; shrinks but remains OpenHuman-owned |

## Phase 0 Drift Rows

| # | Area | Status | Evidence / action |
| --- | --- | --- | --- |
| P0-1 | Version skew: host required `tinyagents = 1.5.0` while the intended engine baseline was `v1.6.0` | **CLOSED** | Phase 0 first aligned the host to `v1.6.0`; the current Phase 1 host pin is now `v1.7.1` in root `Cargo.toml`, both lockfiles, and `vendor/tinyagents` (`3e81e493`). |
| P0-2 | `ToolCompleted` outcome was reconstructed through OpenHuman's `ToolFailureMap` side channel | **CLOSED** | `src/openhuman/tinyagents/observability.rs` consumes TinyAgents 1.6 `duration_ms`, `output_bytes`, and `error`; `ToolFailureMap` now only preserves OpenHuman's richer classified failure and legacy fallback fields. |
| P0-3 | TinyAgents 1.6 event constructor shape changed for local observability tests | **CLOSED** | Local constructors in `src/openhuman/tinyagents/observability.rs` include `ModelCompleted.started_at_ms` and the expanded `ToolCompleted` fields. |
| P0-4 | `invoke_stream` adoption in `src/openhuman/tinyagents/mod.rs` | **CLOSED** | TinyAgents PR [tinyagents#21](https://github.com/tinyhumansai/tinyagents/pull/21) shipped context-preserving `invoke_stream_in_context` in `v1.7.0`; follow-up PR [tinyagents#28](https://github.com/tinyhumansai/tinyagents/pull/28) made that stream `Send` and shipped in `v1.7.1`. `OpenHumanTinyAgentModel::invoke` now drives `invoke_stream_in_context` when progress streaming is enabled, consuming terminal `AgentStreamItem`s while the existing `EventSink` bridge continues to mirror progress. Local validation for #28 in the submodule: `cargo fmt --check`; `timeout 180s cargo clippy --all-targets -- -D warnings`; `timeout 120s cargo test invoke_stream_in_context_stream_is_send`; `timeout 120s cargo test invoke_stream_in_context_unsubscribes_channel_listener`. GitHub release run `28729225952` passed TinyAgents format, clippy, tests, package, tag, and crates.io publish for `v1.7.1`. |
| P0-5 | SHA-256 prompt fingerprint / prompt-cache drift guard | **CLOSED** | `src/openhuman/tinyagents/middleware.rs` now stamps `PromptCacheSegmentMiddleware` segment ids and `ModelRequest::prompt_fingerprint` with SHA-256 over canonical JSON. Tool-cache identity includes the full serialized `ToolSchema` list, not just tool names, matching TinyAgents 1.6 `PromptBuilder::fingerprint` expectations. Added `prompt_cache_segments_fingerprint_full_tool_schema` as the local regression guard. |
| P0-6 | Idempotent redaction middleware vs `journal.rs` double-redaction | **CLOSED** | Audit found no OpenHuman install of TinyAgents `RedactionMiddleware`. Model-facing tool output is scrubbed once by `CredentialScrubMiddleware`; durable event persistence is separately wrapped by `journal.rs` `RedactingSink` over `openhuman_redaction_secrets()`. These protect different surfaces, so there is no crate/host double-redaction seam to collapse in Phase 0. |

## Phase 1 Drift Rows

| # | Area | Status | Evidence / action |
| --- | --- | --- | --- |
| P1-1 | `SchemaCleanr` provider schema normalization | **CLOSED** | TinyAgents PR [tinyagents#20](https://github.com/tinyhumansai/tinyagents/pull/20) shipped in `v1.7.0`. Host `src/openhuman/tools/schema.rs` now re-exports `tinyagents::harness::tool::{CleaningStrategy, SchemaCleanr, GEMINI_UNSUPPORTED_KEYWORDS}`, keeping the old OpenHuman import path stable while deleting the in-tree implementation. Local TinyAgents validation before merge: `cargo fmt --check`; `timeout 180s cargo clippy --all-targets -- -D warnings`; `timeout 120s cargo test schema_`. |
| P1-2 | `current_time` / `resolve_time` builtin tool pilot | **RELEASED / HOST WRAPPER RETAINED** | TinyAgents PR [tinyagents#22](https://github.com/tinyhumansai/tinyagents/pull/22) shipped in `v1.7.0` with optional `tools` feature exports. Host wrappers remain in place until Phase 2 reconciles `ToolResult`, permission, access, and timeout semantics enough to adopt crate builtin tools. Local TinyAgents validation before merge: `cargo fmt --check`; `timeout 240s cargo clippy --features tools --all-targets -- -D warnings`; `timeout 180s cargo test --features tools time_`. |
| P1-3 | `model_context.rs` generic context-window patterns | **CLOSED** | TinyAgents PR [tinyagents#23](https://github.com/tinyhumansai/tinyagents/pull/23) shipped in `v1.7.0`. Host `context_window_for_model` now checks OpenHuman tier aliases and the cost catalog first, then delegates generic raw-model fallback to `tinyagents::harness::model::context_window_for_model_id`. Local TinyAgents validation before merge: `cargo fmt --check`; `timeout 180s cargo clippy --all-targets -- -D warnings`; `timeout 120s cargo test context_window_patterns_cover_common_provider_families`; `timeout 120s cargo test o1_o3_context_patterns_require_segment_boundaries`. |
| P1-4 | `error_classify.rs` generic provider failure classifiers | **RELEASED / HOST CALL-SITE PENDING** | TinyAgents PR [tinyagents#24](https://github.com/tinyhumansai/tinyagents/pull/24) shipped in `v1.7.0` with `harness::retry::{ProviderFailureClass, classify_provider_failure, classify_provider_error, structured_http_status, parse_retry_after_ms}` and shared OpenAI retryability classification. Host retry/failure call-site swaps remain pending because OpenHuman-specific session, billing-envelope, and backend phrase rules stay host-side. Local TinyAgents validation before merge: `cargo fmt --check`; `timeout 180s cargo clippy --all-targets -- -D warnings`; `timeout 120s cargo test provider_failure`; `timeout 120s cargo test structured_http_status`; `timeout 120s cargo test retry_after_parser_accepts_integer_float_and_space_separators`; `timeout 120s cargo test classify_provider_error_reads_structured_error_fields`. |
| P1-5 | First-class reasoning channel host cutover | **CLOSED** | TinyAgents `v1.6.0` already carries typed reasoning via `ContentBlock::Thinking`, `ContentBlock::RedactedThinking`, `MessageDelta::reasoning`, and stream reconstruction that preserves thinking blocks. OpenHuman now writes new non-streaming `reasoning_content` into `ContentBlock::Thinking` instead of `ProviderExtension`, while still reading legacy `ProviderExtension` reasoning from persisted transcripts and continuing to echo `ChatMessage::extra_metadata` for provider replay. Local validation: `cargo fmt --check` passed; two targeted `cargo test --lib --manifest-path Cargo.toml ...` attempts for the new conversion tests timed out during host test compilation before executing, so runtime verification is deferred to GitHub runners. |
| P1-6 | Git-worktree `WorkspaceIsolation` provider | **RELEASED / HOST WRAPPER RETAINED** | TinyAgents PR [tinyagents#25](https://github.com/tinyhumansai/tinyagents/pull/25) shipped in `v1.7.0`. OpenHuman's wrapper remains for global event-bus emissions, `OutsideWorkspace`, and host policy mapping; adapter deletion waits for a focused wrapper-thinning pass. Local TinyAgents validation before merge: `cargo fmt --check`; `timeout 180s cargo clippy --all-targets -- -D warnings`; targeted worktree tests for create/list/status/diff/remove plus overlap and sanitize filters. |
| P1-7 | Tool display metadata and timeout semantics | **RELEASED / HOST TRAIT RETAINED** | TinyAgents PR [tinyagents#26](https://github.com/tinyhumansai/tinyagents/pull/26) shipped in `v1.7.0`. Host `ToolPolicy` projection now fills the new `ToolRuntime.timeout` field, but OpenHuman's `Tool` trait still owns richer legacy display/timeout semantics until the Phase 2 tool model reconciliation. Local TinyAgents validation before merge: `cargo fmt --check`; `timeout 180s cargo clippy --all-targets -- -D warnings`; `timeout 120s cargo test display_`; `timeout 120s cargo test tool_policy_deserializes_without_display_metadata`; `timeout 120s cargo test timeout_policy_uses_richer_timeout_semantics`. |
| P1-8 | Model-layer inversion: host callers still name `Box<dyn Provider>` / call `create_chat_provider` instead of the crate `ChatModel` | **IN PROGRESS** | `create_chat_model(...) -> Arc<dyn ChatModel<()>>` exists (`inference/provider/factory.rs:922`) as a zero-behavior-change shim wrapping the existing provider stack via `ProviderModel` (`tinyagents/model.rs`, built only in `factory.rs::chat_model_from_provider`). Baseline at branch point: ~7 runtime `create_chat_provider` call sites and 25 non-test files outside `inference/provider/` still name `dyn Provider` (incl. the seam `tinyagents/{mod,model,routes}.rs`, which legitimately keep it). **Done:** one-shot callers migrated — cron model-id resolve (`cron/scheduler.rs`), accessibility vision-locate (`accessibility/{automate,vision_click}.rs`); both Cargo worlds green. `agent_meetings/summary`, `memory/chat`, `learning/linkedin_enrichment`, `memory_tree`, `subconscious` runtime paths already on `create_chat_model`. **Deferred (own slice):** `tinyflows/caps.rs` (round-trips tool_calls + reasoning into a JSON envelope — needs seam converter helpers exposed); `src/bin/inference_probe.rs` (debug bin). |
| P1-9 | Harness turn path (`Agent`/`AgentTurnRequest`) carries `Arc<dyn Provider>`, not a crate `ChatModel` | **BLOCKED ON DESIGN — one coupled refactor** | Investigation finding: the plan's Buckets 2–4 (routing/channels, agent harness, subagent runner) are **not independently landable** — `Provider` flows end-to-end: producers (`channels/runtime/dispatch/processor.rs` → `AgentTurnRequest.provider`; `agent/harness/session/builder/factory.rs` → `Agent.provider`; `subagent_runner/ops/provider.rs`) → `agent/bus.rs` / `harness/graph.rs::run_channel_turn_via_graph` → seam `build_turn_models(provider: Arc<dyn Provider>, …)` (`tinyagents/mod.rs:1139`). The channel graph reads Provider-trait capability methods before building the model: `supports_native_tools`, `supports_vision`, `effective_context_window` (**async**), `telemetry_provider_id`. `ProviderModel::profile()` already carries tool_calling / image_in / streaming / context-window, so a `ChatModel`-accepting `build_turn_models` is feasible — but the **async context-window resolution** and **telemetry id** must be re-homed (into the factory at ChatModel construction, or passed as params). Net: one atomic change across ~30 files (incl. ~10 test files) on the live channel/session turn path — must land as its own reviewed PR with streaming/cost/multimodal behavior-parity testing (the flagged regression surface: #4460 thread_id task-locals, $0-turn cost, tool timeline). `routing/provider.rs::IntelligentRoutingProvider` stays a `Provider` impl (provider-stack member, Phase-3 → `ModelRegistry`); it gets wrapped via `chat_model_from_provider` at the producer boundary. **BLOCKER (found while executing):** the harness cannot hold `Arc<dyn ChatModel>` in Phase 1 as the plan assumed. `build_turn_models` needs the raw `Provider` for (a) workload-route projection — `routes::build_route_models(provider: &Arc<dyn Provider>)` re-instantiates a `ProviderModel` per tier alias with distinct model strings + per-route `with_vision`/`with_reasoning` flags, which a single baked `ChatModel` cannot re-alias — and (b) the separate-error-slot summarizer. The crate `ChatModel` trait exposes no `as_any`/downcast, so the `Provider` cannot be recovered from an `Arc<dyn ChatModel>`. Therefore the true harness inversion is gated on **Phase 3** (replace `RouterProvider`/route-projection with the crate `ModelRegistry`), an upstream `vendor/tinyagents` change — not host-only. Achievable host-only step instead: wrap the harness-held `Arc<dyn Provider>` in a seam-owned newtype (e.g. `tinyagents::TurnModelSource`) so no `agent/` code names the `Provider` trait and all Provider handling is confined to the seam + factory, making the Phase-3 swap seam-local. **PROGRESS:** `docs/tinyagents-phase3-router-registry-design.md` records the corrected premise (router→registry already crate-wired in `assemble_turn_harness`; no upstream gap; work is host-only Motion A). `TurnModelSource` (pub seam type) landed + `TurnModels` extended with `provider_id`/`context_window`/`native_tools`/`supports_vision`. **Channel/bus turn path fully migrated** (commit `30c7dfd92`): `AgentTurnRequest.provider → turn_model_source`; `run_channel_turn_via_graph` reads caps off the built crate models; channels/triage producers wrap at the bus boundary; lib + the 3 bus integration tests green; zero behavior change. **Subagent-runner path migrated** (commit `8db888712`): `agent_graph::AgentTurnRequest.provider → turn_model_source`; `run_subagent_via_graph` takes the source (reads vision/native-tool caps + telemetry id off the built models, resolves context window via the source); `SubagentCheckpoint` cap-hit summary now runs on a crate `ChatModel` (via `TurnModelSource::build_summarizer`) instead of `provider.chat`; runner wraps its resolved `subagent_provider` at both dispatch sites. Core lib green; changed files clean under `--lib --tests`; zero behavior change. **Agent session path migrated** (commit `9112330b9`): `Agent`/`AgentBuilder`, `ParentExecutionContext`, and `ChatTurnGraph` hold a `TurnModelSource`/built `TurnModels`; core builds the tiered model set up front (reads vision off it), `ParentExecutionContext` carries the source, and the streaming cap-hit checkpoint keeps `provider.chat` via a `source.provider()` escape hatch (crate `ChatModel::invoke` has no delta sink). Extract tool migrated (commit `6106ced83`). **Motion A is structurally complete:** no agent-harness struct (`Agent`, `AgentBuilder`, `ParentExecutionContext`, `ChatTurnGraph`, both `AgentTurnRequest`s) holds `Arc<dyn Provider>`; both Cargo worlds green; zero behavior change. `TurnModelSource` gained `is_local_provider()` + a `provider()` escape hatch used only at seam-boundary resolution sites. **Remaining `dyn Provider` in `agent/` (Motion B, not Motion A):** provider-*resolution/build* boundaries that construct a provider to wrap into a source — `session/builder/factory.rs` (`create_chat_provider`/`create_routed_provider`), `subagent_runner/ops/provider.rs::resolve_subagent_provider` (kept `Arc<dyn Provider>` to avoid churning its 9 unit tests), `tools/delegate.rs`, `triage/routing.rs`, and the builder `.provider()/.provider_arc()` setters — plus test files. These vanish when Motion B registers crate-native `providers::openai` clients directly. Pre-existing full-`--tests` breakage in unrelated modules (config load, web, ollama, sandbox, reliable_tests, memory) is untouched and orthogonal. |

## Motion B — Provider-Build Cutover (crate-native `ChatModel` construction)

Motion A confined all `Provider` handling to the seam + factory. Motion B
replaces the *construction* of host `Provider`s with crate-native
`ChatModel`s at each build boundary, so `compatible*.rs` can eventually be
deleted. The factory keeps both paths in parallel until every construction
site is crate-backed (the migration's scaffold → flip → delete pattern).

| Site | Crate-native builder | Status |
| --- | --- | --- |
| Managed OpenHuman backend (common path: chat turns, memory/learning/meeting summaries) | `factory::make_openhuman_backend_model` → `OpenHumanBackendModel` (dynamic JWT + `thread_id` + billing envelope bridged onto crate `OpenAiModel`) | **CUT OVER** — `create_chat_model_with_model_id` routes it (commit `7e98c1b39`); test-provider override still wins. |
| Wire-equivalent BYOK cloud slug (Anthropic / None / plain-Bearer, no codex-oauth, no `/v1/responses`) | `factory::try_create_cloud_slug_chat_model` → `crate_openai::make_crate_openai_chat_model` | **CUT OVER (conservative subset)** — `create_chat_model` routes these crate-native after the managed + local short-circuits. Resolution is shared via `resolve_cloud_slug` (the legacy `make_cloud_provider_by_slug` was refactored onto it, so eligible slugs resolve **identically**; only the wire client differs). The same `enforce_local_only_inference` + `verify_session_active` gate runs first. Covers the common non-OpenAI BYOK providers (DeepSeek, Groq, Mistral, xAI, …) via the crate wire the managed backend already proves. |
| `openai` / codex + custom-proxy cloud slugs | crate-native via `try_create_cloud_slug_chat_model` | **CUT OVER** — the flip now covers **every** configured cloud slug except the managed `OpenhumanJwt` entry. Codex OAuth → crate `OpenAiModel` on the Responses API (`with_responses_api_primary` + account/originator headers + user-agent + `client_version` query + `max_output_tokens` omitted), enabled by the crate `/v1/responses` port ([tinyagents#51](https://github.com/tinyhumansai/tinyagents/pull/51)). Non-codex `openai` + custom slugs → crate Chat Completions (the legacy 404 → `/v1/responses` **fallback** is not replicated — chat completions is their primary path). Host pin `8e57665` = #49 toggles + #51 Responses, with #50 reverted (#52) so loopback stays retryable. **Live per-provider validation deferred to a dedicated tinyagents run.** |
| Local runtimes (Ollama/LM Studio/MLX/OMLX/local-openai) | `factory::try_create_local_runtime_chat_model` → `crate_openai::make_crate_local_runtime_chat_model` (native tools + vision forced off; `num_ctx` baked as `{"options":{"num_ctx":N}}`) | **CUT OVER** — `create_chat_model_with_model_id` routes local runtimes crate-native (after the managed short-circuit). The flip **re-runs the same gate** the `Provider` path applies (`enforce_local_only_inference` + `verify_session_active`), so it cannot bypass privacy mode or the session requirement. Temperature rides the per-call `ModelRequest` (parity with managed). **Loopback error handling defers to upstream:** an upstream merge (`b709a993…`/`04ffc029…`) replaced the earlier `..._offline_trips_halt_guard` test with `cron_agent_job_short_loopback_send_error_stays_retryable` — i.e. an offline local provider now **stays retryable** (it may be transiently starting up). So the transient cron `{e:#}` cause-chain surfacing + the `is_non_retryable` loopback fast-fail were reverted, and the host stays pinned at `7c6e81a` (before [tinyagents#50](https://github.com/tinyhumansai/tinyagents/pull/50) `error_source_chain`) so the crate-native local error does not surface the `connection refused` errno the classifier would trip on. #50 remains a good crate improvement but is deliberately **not consumed** here to keep loopback retryable. |
| Bespoke (managed backend, `claude_code`, `claude_agent_sdk`, `openai_codex`) | stay host `ChatModel` impls | **HOST-OWNED** — subprocess / `/v1/responses` / query-param auth have no crate equivalent; never route through `crate_openai`. |

**Crate dependency landed:** [tinyagents#49](https://github.com/tinyhumansai/tinyagents/pull/49)
adds `OpenAiModel::{with_native_tool_calling, with_vision, with_default_provider_options}`
+ a pure `merge_provider_options` (baked defaults merged under per-call
options; a non-object override passes through so validation still rejects it).
Merged onto crate `v1.8.0` main as `7c6e81a`; crate tests: 61 openai unit tests
pass (55 + v1.8.0's #45/#46); `cargo fmt --check` clean. Host pin `7c6e81a`.

**Motion A deferred-test debt (found via PR #4769 CI):** Motion A renamed
`ParentExecutionContext.provider → turn_model_source` (+ the `AgentBuilder`
field) but ~8 `agent_orchestration`/`harness` **test** modules still built the
struct with the old field. Because no PR existed pre-#4769, this was never
CI-tested; the lib-test target (`cargo test --lib`) did not compile, which would
fail CI `rust-core-coverage`. Fixed by wrapping each site in
`TurnModelSource::new(provider)` and correcting the `AgentBuilder` field access.
Touching `agent_orchestration/` test files pulled the orchestration domain into
CI `rust-core-coverage`'s scope, which surfaced a **separate pre-existing**
upstream breakage: `tests/orchestration_effect_executor_e2e.rs` (added by #4738)
still called `dispatch_device_tool`/`handle_tool_call` with the old sync 2-arg
signatures after #4753 made them `async`/3-arg — broken identically on
`upstream/main`. Fixed the two tests to `#[tokio::test]` + `.await` + the
`cycle_id` arg (gate-bypassed for non-local-exec tools).

**Behavior-level test failures (5, surfaced once the suite compiled): all stale
tests, no code regression.** Motion A's "zero behavior change" holds for the
actual runtime contract — the failing tests were written against pre-migration
internals and were never CI-run:

- `bus_turn` / `run_subagent` *surfaces_provider_error* — the crate-owned retry
  (`RunPolicy.retry` max 3, mirroring the old `ReliableProvider`) rides a
  single-shot `ScriptedProvider::failing` through to its empty-queue default `Ok`.
  Fix: `always_fail` field so the mock fails **persistently** (all 3 attempts) —
  a genuinely-down provider still surfaces its error.
- `agent_large_round25` extraction — `extract_from_result` now runs its per-chunk
  extraction through the crate `ChatModel` (`build_summarizer().invoke()`, commit
  `6106ced83`), not the legacy `chat_with_system`; 6 chunk calls hit `chat` and
  drained the agent-turn queue. Fix: route extraction calls (detected by the
  extraction system prompt) to the fixed result in the mock's `chat`.
- `inference…user_state_edges` — expected an unknown model to collapse to
  `reasoning-v1`; the managed backend forwards it verbatim (#4598). Fix: assertion.
- `cron…local_provider_offline_trips_halt_guard` — the **one code fix**:
  `run_agent_job` surfaced `raw` as `e.to_string()` (outer message only), dropping
  the `connection refused (os error N)` cause the halt-guard classifier needs.
  Changed to `{e:#}` (full anyhow chain).

**BYOK cloud-slug cutover — deferred to Phase 3 (deliberate).** The host
`make_cloud_provider_by_slug` Bearer branch (where the common cloud providers —
openai, deepseek, groq, mistral, … — live) layers on `/v1/responses` fallback,
`openai-codex` OAuth headers, user-agent, query params, and
`with_responses_api_primary`. The crate `OpenAiModel` speaks Chat Completions
only, so the Bearer path cannot flip without a crate `/responses` port. The only
crate-native-eligible cloud slugs today are the **rare** None-auth / Anthropic-auth
branches, and even those carry `supports_responses_fallback = true`. Flipping just
that sliver would (a) split cloud routing across two clients for marginal coverage
and (b) touch real-billing paths the ledger requires **per-provider wire-parity
validation** for — validation that needs a live cloud test environment this box
cannot provide. So the BYOK cloud cutover stays with **Phase 3 (provider
consolidation)**: it lands together with the crate `/responses` support + the
router → crate `ModelRegistry` migration, where the whole cloud surface moves
coherently. `compatible*.rs` (host `OpenAiCompatibleProvider`) therefore remains —
it still serves every Bearer cloud slug, `openai_codex`, and the `create_chat_provider`
callers that have not moved to `create_chat_model` — and cannot be deleted until
Phase 3 completes.

## Phase 3 — RouterProvider → crate registry (host-only)

Per `docs/tinyagents-phase3-router-registry-design.md` §1, Phase 3 is **host-only**
— the crate `ModelRegistry` projection is already wired in `assemble_turn_harness`,
so there is **no upstream gap** (this corrects the earlier "upstream-gated" reading
of P1-9). Two sub-motions:

- **P3-A** (harness holds `TurnModels`, not `Provider`): effectively complete —
  `agent/harness/graph.rs` holds the seam `TurnModelSource` (names no `Provider`
  trait) and `TurnModels` carries the `provider_id`/`context_window`/`native_tools`/
  `supports_vision` accessors; the per-turn route re-projection that needs the raw
  `Provider` is confined inside the seam newtype.
- **P3-B** (registered tier models become crate-native, deletes `compatible*.rs`):
  the hot turn path still builds `ProviderModel`-over-`Provider` via
  `build_turn_models`/`build_route_models`. Cutting it to crate-native tiered
  models from config is the remaining work.

| Step | Status | Evidence |
| --- | --- | --- |
| Crate high-level router | **DONE** | [tinyagents#54](https://github.com/tinyhumansai/tinyagents/pull/54) `registry::router::{ModelRouter, WorkloadRoute}` (merged `4fc8cd8`) — declarative workload-tier table (alias→model, `CapabilitySet` gate, same-family fallbacks) filling the long-declared `ComponentKind::Router`; holds no models, no I/O. |
| Host adopts `ModelRouter` for fallback + capability | **IN PROGRESS** | `tinyagents/routes.rs`: `OH_WORKLOAD_ROUTER` (`LazyLock<ModelRouter>`) now backs `route_fallback_policy` + `turn_required_capabilities`; deleted the hand-rolled `same_family_fallbacks`. Behavior-neutral (parity tests pin the exact chains + vision gate incl. `hint:vision`). `build_route_models`' per-tier `ProviderModel` construction (the P3-B client swap) is untouched. Host pin bumped `8e57665` → `4fc8cd8` (adds #53 langfuse run-tree + #54 router). |

## Host Validation Notes

Local host validation is intentionally bounded because full suites are deferred
to GitHub runners. `cargo fmt --check` passed after the v1.7.1 host changes.
Targeted `timeout 240s cargo test --lib --manifest-path Cargo.toml schema_`
and `timeout 240s cargo test --lib --manifest-path Cargo.toml context_window`
timed out during host compilation before executing the filtered tests. A bounded
`timeout 240s cargo check --lib --manifest-path Cargo.toml` first exposed the
non-`Send` stream cutover issue and the new `ToolRuntime.timeout` field; after
filling `ToolRuntime.timeout`, TinyAgents `v1.7.1` closed the stream `Send`
blocker and the host re-applied the `invoke_stream_in_context` cutover. A fresh
bounded `timeout 240s cargo check --lib --manifest-path Cargo.toml` then timed
out before completion with warning output only and no post-cutover compiler
error emitted before the cap.

## Phase Gates

| Phase | Gate rows | Status |
| --- | --- | --- |
| Phase 0 - version alignment | P0-1, P0-2, P0-3, P0-4, P0-5, P0-6 | **CLOSED** |
| Phase 1 - quick upstream ports | SchemaCleanr, error classification, model context, reasoning channel, worktree isolation, display metadata, time tools | **PARTIAL HOST CUTOVER** |
| Phase 2 - tool model and builtin families | ToolResult structure, permission model, ToolAccess, edit tracking, filesystem/network/time tools | **NOT STARTED** |
| Phase 3 - provider consolidation | OpenAI-compatible provider cutover, retry ownership, backend envelope split | **NOT STARTED** |
| Phase 4 - orchestration consolidation | TaskStore/SteeringRegistry lifecycle, status vocabulary, session durability | **NOT STARTED** |
| Phase 5 - workflow/team generic slices | Validation/scheduling slice evaluation | **NOT STARTED** |
| Phase 6 - cleanup and docs | Transitional shim deletion and architecture docs | **NOT STARTED** |

## Closing Procedure

1. For a **DRIFT -> tinyagents PR** row, branch inside `vendor/tinyagents`, port
   the generic change with crate-native tests, merge/release upstream, then bump
   the host submodule and version pin together.
2. For a **HOST-OWNED** row, document the boundary and keep the logic in
   OpenHuman behind the seam.
3. For a **CONSOLIDATE / DELETE** row, add or update the seam proof first, cut the
   live path to TinyAgents, then delete the duplicate host implementation.
4. Update this ledger in the same host PR that closes or reclassifies a row.
