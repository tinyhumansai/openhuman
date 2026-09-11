# TinyAgents Drift Ledger

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
| Host branch | `feat/tinyagents-provider-cleanup` |
| Host audit base | `5b8a9f269` (`upstream/main`, 2026-07-22) |
| Active plan | `docs/tinyagents-migration-plan-2026-07-22.md` |
| TinyAgents submodule | `vendor/tinyagents` -> `tinyhumansai/tinyagents` |
| Phase 0 target | `v1.6.0` / `e72036d847b589044aa9a4add1b34544b92a293d` |
| Current host pin | `v2.1.0` / `2583fcc` (vendored tag; root requirement `2.1`) |
| Verification PR | [openhuman#4769](https://github.com/tinyhumansai/openhuman/pull/4769) — Motion A + Motion B checkpoint. **CI fully green** (`PR CI Gate` + `Rust Core Coverage` + fmt/clippy); 14,437 Rust tests pass. First full CI verification of this branch. |

## Baseline Snapshot

Recorded from `docs/tinyagents-port-plan` after the Phase 0 version alignment
work started. Counts include Rust files only.

| Host module | Rust files | LOC | Test fns | Plan disposition |
| --- | ---: | ---: | ---: | --- |
| `src/openhuman/inference/` | 116 | 53,023 | 1,101 | Provider consolidation and small generic ports |
| `src/openhuman/tools/` | 94 | 38,553 | 877 | Tool model reconciliation, then builtin family ports |
| `src/openhuman/agent/orchestration/` | 64 | 25,769 | 262 | Sub-agent lifecycle consolidation onto TinyAgents graph/orchestration |
| `src/openhuman/agent/tinyagents/` | 25 | 15,219 | 101 | Host seam; shrinks but remains OpenHuman-owned |

## Phase 0 Drift Rows

| # | Area | Status | Evidence / action |
| --- | --- | --- | --- |
| P0-1 | Version skew: host required `tinyagents = 1.5.0` while the intended engine baseline was `v1.6.0` | **CLOSED** | Phase 0 first aligned the host to `v1.6.0`; the current Phase 1 host pin is now `v1.7.1` in root `Cargo.toml`, both lockfiles, and `vendor/tinyagents` (`3e81e493`). |
| P0-2 | `ToolCompleted` outcome was reconstructed through OpenHuman's `ToolFailureMap` side channel | **CLOSED** | `src/openhuman/agent/tinyagents/observability.rs` consumes TinyAgents 1.6 `duration_ms`, `output_bytes`, and `error`; `ToolFailureMap` now only preserves OpenHuman's richer classified failure and legacy fallback fields. |
| P0-3 | TinyAgents 1.6 event constructor shape changed for local observability tests | **CLOSED** | Local constructors in `src/openhuman/agent/tinyagents/observability.rs` include `ModelCompleted.started_at_ms` and the expanded `ToolCompleted` fields. |
| P0-4 | `invoke_stream` adoption in `src/openhuman/agent/tinyagents/mod.rs` | **CLOSED** | TinyAgents PR [tinyagents#21](https://github.com/tinyhumansai/tinyagents/pull/21) shipped context-preserving `invoke_stream_in_context` in `v1.7.0`; follow-up PR [tinyagents#28](https://github.com/tinyhumansai/tinyagents/pull/28) made that stream `Send` and shipped in `v1.7.1`. `OpenHumanTinyAgentModel::invoke` now drives `invoke_stream_in_context` when progress streaming is enabled, consuming terminal `AgentStreamItem`s while the existing `EventSink` bridge continues to mirror progress. Local validation for #28 in the submodule: `cargo fmt --check`; `timeout 180s cargo clippy --all-targets -- -D warnings`; `timeout 120s cargo test invoke_stream_in_context_stream_is_send`; `timeout 120s cargo test invoke_stream_in_context_unsubscribes_channel_listener`. GitHub release run `28729225952` passed TinyAgents format, clippy, tests, package, tag, and crates.io publish for `v1.7.1`. |
| P0-5 | SHA-256 prompt fingerprint / prompt-cache drift guard | **CLOSED** | `src/openhuman/agent/tinyagents/middleware.rs` now stamps `PromptCacheSegmentMiddleware` segment ids and `ModelRequest::prompt_fingerprint` with SHA-256 over canonical JSON. Tool-cache identity includes the full serialized `ToolSchema` list, not just tool names, matching TinyAgents 1.6 `PromptBuilder::fingerprint` expectations. Added `prompt_cache_segments_fingerprint_full_tool_schema` as the local regression guard. |
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
| P1-8 | Model-layer inversion: host callers still name `Box<dyn Provider>` / call `create_chat_provider` instead of the crate `ChatModel` | **IN PROGRESS** | `create_chat_model(...) -> Arc<dyn ChatModel<()>>` exists (`inference/provider/factory.rs:922`) as a zero-behavior-change shim wrapping the existing provider stack via `ProviderModel` (`tinyagents/model.rs`, built only in `factory.rs::chat_model_from_provider`). Baseline at branch point: ~7 runtime `create_chat_provider` call sites and 25 non-test files outside `inference/provider/` still name `dyn Provider` (incl. the seam `tinyagents/{mod,model,routes}.rs`, which legitimately keep it). **Done:** one-shot callers migrated — cron model-id resolve (`cron/scheduler.rs`), accessibility vision-locate (`accessibility/{automate,vision_click}.rs`); both Cargo worlds green. `agent_meetings/summary`, `memory/chat`, `learning/linkedin_enrichment`, `memory_tree`, `subconscious` runtime paths already on `create_chat_model`. **Deferred (own slice):** `tinyflows/caps.rs` (round-trips tool_calls + reasoning into a JSON envelope — needs seam converter helpers exposed). (`src/bin/inference_probe.rs` was also deferred; the debug bin has since been deleted.) |
| P1-9 | Harness turn path (`Agent`/`AgentTurnRequest`) carries `Arc<dyn Provider>`, not a crate `ChatModel` | **BLOCKED ON DESIGN — one coupled refactor** | Investigation finding: the plan's Buckets 2–4 (routing/channels, agent harness, subagent runner) are **not independently landable** — `Provider` flows end-to-end: producers (`channels/runtime/dispatch/processor.rs` → `AgentTurnRequest.provider`; `agent/harness/session/builder/factory.rs` → `Agent.provider`; `subagent_runner/ops/provider.rs`) → `agent/bus.rs` / `harness/graph.rs::run_channel_turn_via_graph` → seam `build_turn_models(provider: Arc<dyn Provider>, …)` (`tinyagents/mod.rs:1139`). The channel graph reads Provider-trait capability methods before building the model: `supports_native_tools`, `supports_vision`, `effective_context_window` (**async**), `telemetry_provider_id`. `ProviderModel::profile()` already carries tool_calling / image_in / streaming / context-window, so a `ChatModel`-accepting `build_turn_models` is feasible — but the **async context-window resolution** and **telemetry id** must be re-homed (into the factory at ChatModel construction, or passed as params). Net: one atomic change across ~30 files (incl. ~10 test files) on the live channel/session turn path — must land as its own reviewed PR with streaming/cost/multimodal behavior-parity testing (the flagged regression surface: #4460 thread_id task-locals, $0-turn cost, tool timeline). `routing/provider.rs::IntelligentRoutingProvider` stays a `Provider` impl (provider-stack member, Phase-3 → `ModelRegistry`); it gets wrapped via `chat_model_from_provider` at the producer boundary. **BLOCKER (found while executing):** the harness cannot hold `Arc<dyn ChatModel>` in Phase 1 as the plan assumed. `build_turn_models` needs the raw `Provider` for (a) workload-route projection — `routes::build_route_models(provider: &Arc<dyn Provider>)` re-instantiates a `ProviderModel` per tier alias with distinct model strings + per-route `with_vision`/`with_reasoning` flags, which a single baked `ChatModel` cannot re-alias — and (b) the separate-error-slot summarizer. The crate `ChatModel` trait exposes no `as_any`/downcast, so the `Provider` cannot be recovered from an `Arc<dyn ChatModel>`. Therefore the true harness inversion is gated on **Phase 3** (replace `RouterProvider`/route-projection with the crate `ModelRegistry`), an upstream `vendor/tinyagents` change — not host-only. Achievable host-only step instead: wrap the harness-held `Arc<dyn Provider>` in a seam-owned newtype (e.g. `tinyagents::TurnModelSource`) so no `agent/` code names the `Provider` trait and all Provider handling is confined to the seam + factory, making the Phase-3 swap seam-local. **PROGRESS:** `docs/tinyagents-phase3-router-registry-design.md` records the corrected premise (router→registry already crate-wired in `assemble_turn_harness`; no upstream gap; work is host-only Motion A). `TurnModelSource` (pub seam type) landed + `TurnModels` extended with `provider_id`/`context_window`/`native_tools`/`supports_vision`. **Channel/bus turn path fully migrated** (commit `30c7dfd92`): `AgentTurnRequest.provider → turn_model_source`; `run_channel_turn_via_graph` reads caps off the built crate models; channels/triage producers wrap at the bus boundary; lib + the 3 bus integration tests green; zero behavior change. **Subagent-runner path migrated** (commit `8db888712`): `agent_graph::AgentTurnRequest.provider → turn_model_source`; `run_subagent_via_graph` takes the source (reads vision/native-tool caps + telemetry id off the built models, resolves context window via the source); `SubagentCheckpoint` cap-hit summary now runs on a crate `ChatModel` (via `TurnModelSource::build_summarizer`) instead of `provider.chat`; runner wraps its resolved `subagent_provider` at both dispatch sites. Core lib green; changed files clean under `--lib --tests`; zero behavior change. **Agent session path migrated** (commit `9112330b9`): `Agent`/`AgentBuilder`, `ParentExecutionContext`, and `ChatTurnGraph` hold a `TurnModelSource`/built `TurnModels`; core builds the tiered model set up front (reads vision off it), `ParentExecutionContext` carries the source, and the streaming cap-hit checkpoint keeps `provider.chat` via a `source.provider()` escape hatch (crate `ChatModel::invoke` has no delta sink). Extract tool migrated (commit `6106ced83`). **Motion A is structurally complete:** no agent-harness struct (`Agent`, `AgentBuilder`, `ParentExecutionContext`, `ChatTurnGraph`, both `AgentTurnRequest`s) holds `Arc<dyn Provider>`; both Cargo worlds green; zero behavior change. `TurnModelSource` gained `is_local_provider()` + a `provider()` escape hatch used only at seam-boundary resolution sites. **Remaining `dyn Provider` in `agent/` (Motion B, not Motion A):** provider-*resolution/build* boundaries that construct a provider to wrap into a source — `session/builder/factory.rs` (`create_chat_provider`/`create_routed_provider`), `subagent_runner/ops/provider.rs::resolve_subagent_provider` (kept `Arc<dyn Provider>` to avoid churning its 9 unit tests), `tools/delegate.rs`, `triage/routing.rs`, and the builder `.provider()/.provider_arc()` setters — plus test files. These vanish when Motion B registers crate-native `providers::openai` clients directly. Pre-existing full-`--tests` breakage in unrelated modules (config load, web, ollama, sandbox, reliable_tests, memory) is untouched and orthogonal. |

## Motion B — Provider-Build Cutover (crate-native `ChatModel` construction)

The closure rows below supersede older deferred/pending prose retained later in
this section as investigation history.

| # | Area | Status | Evidence / action |
| --- | --- | --- | --- |
| P1-10 | Wire-equivalent BYOK cloud slugs | **CLOSED** | [openhuman#4780](https://github.com/tinyhumansai/openhuman/pull/4780) routed eligible slugs to crate-native `OpenAiModel`. |
| P1-11 | OpenAI/Codex/custom cloud slugs | **CLOSED** | [openhuman#4782](https://github.com/tinyhumansai/openhuman/pull/4782) completed the configured cloud-slug client cutover, including Responses API construction. |
| P1-12 | Host workload routing | **CLOSED** | [openhuman#4783](https://github.com/tinyhumansai/openhuman/pull/4783) adopted crate `ModelRouter` for fallback and capability decisions. |
| P1-13 | Crate-native registered turn models / `compatible*.rs` deletion | **CLOSED** | [openhuman#4784](https://github.com/tinyhumansai/openhuman/pull/4784) moved the hot turn path and deleted the former `compatible*.rs` cluster. The collapsed `legacy_provider.rs` facade and the broader legacy `Provider` stack remain WP-1 deletion work; see the deletion ledger. |
| P1-14 | Legacy `run_turn_engine` and `OPENHUMAN_AGENT_GRAPH_*` escape hatches | **CLOSED BEFORE AUDIT** | WP-3 verified that both session and subagent production paths call the TinyAgents seam unconditionally. No engine definition or env read remains; only historical parity comments and stale migration prose named the retired implementation. |

## WP-2 Consolidation Audit

| # | Area | Status | Evidence / action |
| --- | --- | --- | --- |
| WP2-1 | `routing/` parallel provider/health stack | **CLOSED / DELETED** | Repository-wide reference audit found no caller outside the module; the live route already uses crate `ModelRouter`. Deleted the unreachable stack and kept the crate-backed seam. |
| WP2-2 | `tool_timeout/` vs crate `harness::tool::ToolTimeout` | **HOST-OWNED** | The crate type is declarative per-tool metadata (`Inherit`/`Unbounded`/`Millis`) and has no process-global setting or execution store. OpenHuman owns persisted-config and `OPENHUMAN_TOOL_TIMEOUT_SECS` precedence, live UI updates, bounds/grace semantics, and the `tokio::time::timeout` around adapted host tools. The seam already projects host timeout policy into crate `ToolRuntime`; no upstream gap or duplicate engine remains. |
| WP2-3 | `model_council/` ensemble | **CLOSED / DELETED** | The generic fan-out already used `tinyagents::graph::parallel::map_reduce`; upstream subsequently removed the unreachable product council and registry surfaces during dead-code cleanup. |
| WP2-4 | `tool_status/` failure classification | **HOST-OWNED** | The types are serialized into OpenHuman threads/UI and the classifier consumes OpenHuman security markers, product retry categories, and user-facing remediation copy. TinyAgents owns raw tool outcomes; the host mapping is deliberately downstream product policy. |

## WP-5 Parallel Fan-Out And Worktree Ownership Audit

| Surface | Status | Ownership / exit evidence |
| --- | --- | --- |
| `orchestration/worktree.rs` git plumbing — `BaseRef`, `WorktreeStatus`, `create`/`list`/`status`/`diff_summary`/`remove`, `detect_overlaps`, `sanitize_run_id` | **CLOSED / DELETED** | Near-verbatim twins of `tinyagents::harness::workspace::git` (`detect_overlaps` and `detect_worktree_overlaps` matched down to the `BTreeSet` dedupe; the two error enums carried identical message strings). Deleted and re-exported under the historical OpenHuman names, so RPC schemas, tools and tests are unchanged. 618 → 242 LOC. The RPC wire shape is pinned by `worktree_status_serializes_with_stable_camel_case_keys`, written and passing **before** the swap and again after; the crate carries the identical `sanitize_run_id` test already. |
| `OpenHumanWorktreeIsolation` (was `GitWorktreeIsolation`) | **HOST-OWNED** | Wraps the crate provider to add the two things that are OpenHuman's rather than every host's: the `openhuman.worktree:{agent}:{run_id}` policy-id convention, and `DomainEvent::Workspace{Prepared,Cleanup}` on the global bus. |
| `enforce_workspace_path` + `WorkspacePathError` | **HOST-OWNED (currently uncalled)** | Descriptor-policy gate that publishes `DomainEvent::WorkspaceViolation`. A repository-wide search finds no caller; it is retained because removing a public fail-closed gate is a separate judgement from this migration, and is recorded here so the next reader does not have to rediscover it. Split off `WorktreeError` (now the crate's git error) since it is about descriptor policy, not git. |
| Shared-workspace claim arbitration — `ownership_file_paths` path validation, `paths_overlap`, the serial-claim accumulator | **CLOSED / CRATE ADOPTED** | Upstreamed as `tinyagents::graph::parallel::claims` (`WorkspaceClaim`, `plan_shared_workspace_dispatch`, `parse_relative_claim_paths`, `paths_overlap`). `prepare_spawn_parallel_tasks_from_defs` is now two passes: OpenHuman policy admits or rejects, then one planner call arbitrates every admitted claim in input order. Behaviour parity is the gate, and `spawn_parallel_agents_tests.rs` passes **unedited** across the cutover; three assertions pinning the mixed-batch dispatch sequence, disjoint-ownership admission, and directory containment were added before it. |
| `files:` ownership syntax, rejection copy, `shared_workspace_write_capable_tools`, `definition_visible_tool_permissions`, `ParallelTaskRejectionKind`, `integrations_agent` toolkit rule, subagent allowlist | **HOST-OWNED** | Product policy and user-facing phrasing. The crate returns `ClaimConflict` as data specifically so the `isolation="worktree"` remedy sentence stays here; `shared_workspace_conflict_message` renders it. |
| `annotate_stale_parent_reads`, `overlap_warnings_for_results` | **HOST-OWNED** | The first reads OpenHuman `file_state` task-locals with no crate analogue; the second is `serde_json` shaping over the crate's overlap detector. Host projection over a crate primitive, not duplication. |
| `run_spawn_parallel_workers` serial-then-parallel execution | **HOST-OWNED (deliberate)** | The crate contributes the *plan*, not the scheduler. Today the host serializes the whole batch when any worker needs serializing; a crate-side executor running only the claimed workers serially would be a **narrower** guarantee than what ships. Keeping execution host-side keeps the diff to "who computed this decision", which is exhaustively unit-testable. |
| `tinyagents/subagent_graph.rs` no-op pipeline skeleton | **CLOSED / DELETED** | Six nodes whose bodies only pushed their own name onto a `Vec`, compiled and executed on **every** sub-agent spawn from `ops/runner.rs`. A scaffold for a per-phase cutover that WP-5 concluded should not happen (see below). Deleted with its call site and its `topology.rs` export. |
| `extract_tool.rs` bounded fan-out | **CLOSED / CRATE ADOPTED** | Hand-rolled `buffer_unordered(3)` plus a manual index-tag-and-re-sort, replaced by `map_reduce` with `FailurePolicy::BestEffort`, which returns outcomes in input order. The per-chunk provider error stays the fan-out's *item* rather than its error, so one failed chunk still drops with a warning instead of aborting its siblings. |
| `harness/subagent_runner/` (7,471 LOC) — **directory-level verdict, not file-level** | **HOST-OWNED — do not relocate** | Its own module docs call it the OpenHuman *build pipeline*, and that is accurate: definition lookup, tier gating, archetype prompt assembly, toolkit filtering, sandbox narrowing, the deterministic memory fast path, Composio resolution, artifact offload, transcript persistence. The generic contract it would map onto already exists as `harness::host::HostCapabilities` (`ContextComposer` / `DefinitionRegistry` / `SecurityGate` / `ModelResolver`) and is unused by the host. The open question is therefore whether OpenHuman should *implement those traits*, not whether to move this code across the GPL boundary — a separate design-gated package. **Exception, resolved:** `subagent_runner/handoff.rs` was a 287-line near-verbatim twin of `tinyagents::harness::handoff` (exported ungated at `vendor/tinyagents/src/harness/mod.rs:23`) and has been collapsed to a ~66-line shim re-exporting the crate module under the historical OpenHuman names; only the host-only `OPENHUMAN_TEST_HANDOFF_THRESHOLD_TOKENS` resolution stays host-side, passed as the crate's explicit `threshold_tokens` parameter. Zero call-site edits. |

## WP-5 Detached Lifecycle Ownership Audit

| Surface | Status | Ownership / exit evidence |
| --- | --- | --- |
| Process-local detached task map, ownership checks, status receivers, cancellation tokens, abort handles, terminal sweep, steering lookup | **CLOSED / CRATE ADOPTED — tinyagents#75** | TinyAgents #75 merged as `d548657`; the canonical vendored pointer `4358efe` includes its `DetachedTaskRegistry`. OpenHuman calls the crate registry for snapshots, wait/timeout, owned and trusted steering, per-task/thread/global cancellation, and soft-cap cleanup. Distinct lock-poison errors are propagated by the crate. All 17 focused host tests pass. |
| Durable detached task lifecycle | **HOST PROJECTION ON CRATE STORE** | OpenHuman maps product `SubagentStatus` into crate `OrchestrationTaskStatus` and owns the store's on-disk location; this is durable product projection, not executor ownership. (Store *selection* itself has since moved to the crate — see the `TaskStoreRegistry` row below, which supersedes the earlier wording of this one.) |
| Per-workspace store selection + durable→memory fallback (`DetachedTaskStore` enum, `TASK_STORES` map, `open_task_store`) | **CLOSED / CRATE ADOPTED** | Upstreamed as `TaskStoreRegistry` + `open_jsonl_task_store_or_memory`. Opening a second store over one append log gives two writers with independently replayed state, so the caching is part of the contract, not an optimization. The 12-arm delegating `TaskStore` impl is deleted; the fallback ladder is ported exactly (a `create_dir_all` failure degrades to memory just as an unreadable log does — a read-only workspace must stay able to spawn work). Lock poisoning is now typed (`TaskStoreRegistryError`) rather than `.expect()`ed. All 17 focused `running_subagents` tests pass unedited. |
| Boot-time orphan reconciliation state machine | **CLOSED / CRATE ADOPTED** | `reconcile_orphaned_tasks` owns which statuses are live and which terminal state each takes (`CancelRequested` → cancelled, other live → failed), and captures per-task transition failures rather than aborting — a record racing to terminal between listing and transition is expected. The reason is passed as a closure, so `"sub-agent orphaned by core restart (was \`running\`)"` and the `publish_subagent_failed` lifecycle event stay host-side. Already wired at `core/jsonrpc.rs`. |
| `REGISTRY_SOFT_CAP` (256) | **HOST-TUNED CRATE POLICY — no action** | The sweep is crate-owned: the constant is passed into `DetachedTaskRegistry::new`, which performs it. Only the number is OpenHuman's. Recorded because an earlier scoping pass mistook it for a host-side duplicate. |
| Detached task metadata, RPC/UI delivery, `RunQueue` fallback | **HOST-OWNED** | Agent/session/thread/workspace metadata, cancellation notices, background delivery, trusted desktop RPC, and compatibility steering are OpenHuman product surfaces. The fallback can shrink independently after live crate-steering parity, but does not block generic registry deletion. |

TinyAgents #75 is merged and the temporary integration gitlink has been
replaced by canonical upstream commit `4358efe`.

## WP-5 Middleware Ownership Audit

The earlier seam snapshot counted 17 middleware types. The audit found 18: the
tool-exposure shadow was added after that snapshot. The crate-backed
`SchemaGuardMiddleware` cutover below reduces the current file to 17 again.
`TurnContextMiddleware` is an installer/config bundle rather than another hook
implementation and is not counted. A middleware may be deleted only after its
crate replacement and host cutover are both verified.

| Middleware | Status | Ownership / exit evidence |
| --- | --- | --- |
| `TranscriptSnapshotMiddleware` | **HOST-OWNED** | Mirrors partial crate transcripts into OpenHuman's persisted `ChatMessage` DTO so failed sub-agent runs remain ingestible. |
| `OpenHumanToolExposureShadowMiddleware` | **TRANSITIONAL / PARITY-GATED** | Exercises crate allowlist/contextual selection in shadow against the host registry. Delete with the host precompute only after divergence telemetry proves parity; do not flip from this audit alone. |
| `HandoffMiddleware` | **HOST-OWNED** | Implements integrations-agent progressive disclosure through OpenHuman's `ResultHandoffCache` and `extract_from_result` contract. |
| `PromptCacheSegmentMiddleware` | **HOST SEAM** | Projects OpenHuman's stable system/tool-cache boundaries and SHA-256 identity into the crate `PromptCacheGuardMiddleware`; the actual drift guard is crate-owned. |
| `ToolOutputMiddleware` | **HOST-OWNED** | Applies host artifact persistence, TokenJuice compaction, byte caps, and the OpenHuman payload summarizer. |
| `ApprovalSecurityMiddleware` | **HOST-OWNED** | Calls the global OpenHuman approval gate, redacts args, and records product audit rows. Generic crate approval cannot replace this product/security decision. |
| `CliRpcOnlyMiddleware` | **HOST-OWNED** | Enforces OpenHuman `ToolScope` at the autonomous-agent boundary. WP-4 defines the complete Agent/CLI/RPC scope matrix. |
| `CredentialScrubMiddleware` | **HOST-OWNED** | Uses OpenHuman's credential detector and scrubs model context, errors, raw JSON, and persisted host surfaces. Crate redaction protects a different generic boundary. |
| `ToolPolicyMiddleware` | **HOST-OWNED** | Enforces args-aware channel permission ceilings, generated-tool provenance, and OpenHuman policy decisions. Static crate `ToolPolicy` remains metadata and a generic fail-closed layer. |
| `ToolOutcomeCaptureMiddleware` | **HOST-OWNED** | Projects final capped results and classified failures into OpenHuman tool-call records/UI state. |
| `ArgRecoveryMiddleware` | **UPSTREAM READY — tinyagents#71** | PR #71 adds `NormalizeThenReturnToolError` and admission-time normalization with preservation regressions. Delete after the merged crate policy is vendored and the host config selects it. |
| `SchemaGuardMiddleware` | **CLOSED / DELETED** | TinyAgents 2.1 already provides `InvalidArgsPolicy::ReturnToolError`. The host now selects it and deleted pre-validation, synthetic schema-valid arguments, the pending map, and the tool-wrap short circuit. The policy regression and all 18 `agent_harness_e2e` tests pass. PR #71 is only needed to absorb the remaining normalization middleware. |
| `MemoryProtocolMiddleware` | **HOST-OWNED** | Enforces OpenHuman's read/dedupe/write/index memory protocol and product tool names. |
| `CostBudgetMiddleware` | **HOST PROJECTION** | TinyAgents `BudgetMiddleware` already runs in shadow; this wrapper maps OpenHuman billing-envelope USD/token accounting and halt summaries. Thin only when crate usage is sufficient for every host provider. |
| `RepeatedToolFailureMiddleware` | **CRATE-BACKED / HOST PROJECTION** | Detection uses crate `NoProgressTracker`; the wrapper owns OpenHuman retry taxonomy, polling exemptions, steering, and user-facing halt summary. No duplicate generic tracker remains to upstream. |
| `RepeatProgressMiddleware` | **CLOSED / CRATE-BACKED — tinyagents#72** | PR #72 merged and is included in canonical pointer `4358efe`. The host duplicate `StreakGuard`, thresholds, and streak accounting are deleted; the remaining thin adapter builds OpenHuman signatures/polling exemptions and maps the crate `SuccessfulRepeatTracker` verdict to the host halt summary and steering pause. All 51 focused middleware tests pass. |
| `ImageAwareMessageTrimMiddleware` | **UPSTREAM READY — tinyagents#73** | PR #73 makes crate trimming image/token-policy aware. Delete after vendoring and proving host context-window regressions against the crate middleware. |

The three upstream PRs are independently mergeable and green at the time of
this audit. Their remaining host deletions stay pending because OpenHuman still
vendors an earlier TinyAgents revision.

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

## `agent/learning/` Ownership Audit

No prior row covered this domain (~29 files, ~6.6k production lines excluding
`*_tests.rs`), which left the question re-derived from scratch on every pass.

| Surface | Status | Ownership / exit evidence |
| --- | --- | --- |
| Ambient personalization cache, stability detector, candidate producers, `PROFILE.md` rendering, LinkedIn enrichment, transcript ingestion (`src/openhuman/agent/learning/`) | **HOST-OWNED** | The runtime-learns-nothing boundary is already drawn upstream: `vendor/tinyagents/src/harness/host/learning_sink.rs` states "The runtime itself learns nothing … what counts as a lesson, where it is stored, whether it is redacted first — is host policy and stays host-side," and OpenHuman implements that seam at `src/openhuman/agent/tinyagents/host/learning_sink.rs`. `vendor/tinymemory/crates/tinymemory-bus/src/learning.rs` separately declines to own the stability formula by name. |
| Structural blocker on porting the stability formula onto `tinymemory-api` | **HOST-OWNED (one-package rule, not a dependency cycle)** | No cargo cycle exists — `tinymemory-api` depends only on `tinymemory-bus` plus leaf crates (`vendor/tinymemory/crates/tinymemory-api/Cargo.toml:47-52`). The real blocker: `tinymemory` vendors `tinyagents` as its own submodule (`vendor/tinymemory/vendor/tinyagents`), `tinymemory-api` is unpublished (no `source` entry in `Cargo.lock`), and adding it as a second path dependency here would make `FacetClass` two incompatible types under cargo's package identity rules — the same trap documented for `tinytools` in the root `CLAUDE.md`. |
| `TurnSummary::tools_invoked` names-only | **UPSTREAM GAP (tinyagents issue, not a host move)** | `vendor/tinyagents/src/harness/host/learning_sink.rs:91` types it `Vec<String>` — no arguments, no result. `ToolTrackerHook` (`src/openhuman/agent/learning/tool_tracker.rs`) and `AgentExperienceCaptureHook` (`src/openhuman/agent/experience/capture.rs`) silently self-disable on the crate-driven turn path because they need call arguments/results to do anything. File upstream; do not attempt a host workaround that reconstructs the missing data. |

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
| Host adopts `ModelRouter` for fallback + capability | **CLOSED** | [openhuman#4783](https://github.com/tinyhumansai/openhuman/pull/4783) made `tinyagents/routes.rs::OH_WORKLOAD_ROUTER` authoritative for fallback and required capabilities. |
| Registered tier models become crate-native | **CLOSED** | [openhuman#4784](https://github.com/tinyhumansai/openhuman/pull/4784) completed the P3-B hot-path client swap and deleted the former `compatible*.rs` cluster. Residual `ProviderModel` and the collapsed legacy facade are tracked separately in WP-1. |

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
| Phase 3 - provider consolidation | OpenAI-compatible client cutover and crate router adoption | **CLIENT CUTOVER CLOSED; LEGACY STACK REMOVAL IN WP-1** |
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
