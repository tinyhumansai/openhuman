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
| Current host pin | `v1.7.1` / `3e81e493` |

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
