# TinyAgents Migration — Current-State Audit & Consolidated Plan (2026-07-22)

**Status:** audit + plan (supersedes the stale status/phasing in
`tinyagents-port-plan.md`, `tinyagents-inference-migration-plan.md`, and
`tinyagents-phase3-router-registry-design.md`; the drift ledger
`tinyagents-drift-ledger.md` remains the row-level ledger but its anchors are
behind — see §2).
**Scope:** move the remaining generic inference + agent-framework code from
`src/openhuman/` down into the vendored `tinyagents` crate
(`vendor/tinyagents`), delete the in-tree duplicates, migrate/retire the
affected tests, and clean up every dangling doc/code reference left behind by
earlier phases.
**Method:** fresh four-way audit of (1) the crate surface, (2)
`src/openhuman/inference/`, (3) the agent domains + the
`src/openhuman/agent/tinyagents/` seam, (4) the existing docs/tests — all against the
working tree at `main` (`5b8a9f269`, 2026-07-22).

**Execution status:** active on `feat/tinyagents-provider-cleanup`.
WP-0 discovered and corrected an additional cross-crate constraint: once the
vendored manifest honestly reports 2.1, `tinyflows` must also require
TinyAgents 2.1 or Cargo resolves a second 1.9 copy and splits trait identity.
The deletion ledger is
[`tinyagents-full-migration-plan/99-deletion-ledger.md`](tinyagents-full-migration-plan/99-deletion-ledger.md).

---

## 1. Executive summary

The migration is **much further along than any existing doc records**, and the
remaining work is now well-bounded:

- The agent loop has run on `tinyagents` since #4249/#4399; the drift ledger's
  Phase 0 rows are all CLOSED and Phase 1 is mostly closed.
- Since the docs were last touched, PRs **#4769, #4780, #4782, #4783, #4784**
  landed the model-layer inversion: the managed backend, wire-equivalent BYOK
  slugs, and openai/codex/custom slugs are all crate-native `ChatModel`s; the
  crate `ModelRouter` is adopted; and **`compatible*.rs` is already deleted**
  (collapsed into a single `legacy_provider.rs` facade).
- What remains falls into six work packages (§5): finish the model-layer
  cutover and delete the legacy `Provider` stack (~9–10k LOC); record that the
  legacy `run_turn_engine` was already retired; consolidate `routing/`, `tool_timeout/`,
  `tool_status/`, `model_council/` onto crate primitives; reconcile the tool
  model (the one genuinely design-gated package); shrink the seam; and a
  housekeeping package (version-pin reconciliation + broken-link cleanup) that
  should go **first** because the current pin state is misleading.

Nothing in this plan touches the CEF shell, the frontend, or the compile-time
feature gates; the crate stays impossible to gate away (26+ domains consume it).

---

## 2. Ground truth: version pins (WP-0 reconciled)

| Fact | Value | Where |
| --- | --- | --- |
| Host requirement | `tinyagents = { version = "2.1", features = ["sqlite"] }` | root `Cargo.toml:107` |
| Path override | `[patch.crates-io] tinyagents = { path = "vendor/tinyagents" }` | `Cargo.toml:677` |
| Historical fork patch | Removed in WP-0; TinyCortex now declares crates.io `2.1`, so the existing host path patch unifies its dependency too | root and Tauri manifests; TinyCortex #121 |
| Vendored crate | `version = "2.1.0"`, tag `v2.1.0` / `2583fcc`, edition 2024, GPL-3.0-only | `vendor/tinyagents/Cargo.toml`, gitlink |
| Included since the audit | `bytes` bump (#59), **`BarrierRelief` graph fan-in primitive (#62)**, v2.0/v2.1 release commits | submodule history |
| TinyFlows requirement | `tinyagents = "2.1"`; prevents Cargo resolving a second 1.9 crate | `vendor/tinyflows/Cargo.toml` |
| TinyCortex requirement | `tinyagents = "2.1"`; its repo-local patch serves standalone development while host patches select the canonical copy | `vendor/tinycortex/Cargo.toml` |
| Drift ledger | Current pin `v2.1.0` / `2583fcc` | `docs/tinyagents-drift-ledger.md` §Anchors |

WP-0 bumped the gitlink to the released v2.1.0 manifest, regenerated both Cargo
worlds, removed the unused fork-source patch, and aligned TinyFlows and
TinyCortex to 2.1. Before those transitive alignments, correct dependency
resolution introduced separate TinyAgents 1.9 and nested-path 2.0 packages;
the lockfiles now prove the root, TinyFlows, and TinyCortex all use the one
vendored 2.1 package.

---

## 3. What the crate ships (v2.1.0)

Full module map from the crate audit; the items relevant to the remaining work:

- **Model layer:** `harness::model::{ChatModel<State>, ModelRegistry, ProviderError}`
  (`vendor/tinyagents/src/harness/model/types.rs:524,563,450`),
  `MODEL_CONTEXT_PATTERNS` context-window table (`model/mod.rs:51`).
- **OpenAI-compatible client:** `harness::providers::openai::{OpenAiModel, AuthStyle,
  ReasoningTagExtraction}` with SSE streaming, the `/v1/responses` API + codex
  knobs (#51), configurable auth styles/static headers (#44), temperature
  suppression (#47), `merge_system_into_user` (#48), prompt-guided tool calling
  for non-native models (#55), stream→JSON fallback (#56), and the whole #57
  local-model hardening wave (inline `<think>` extraction, 400-degrade of
  `tool_choice`/`json_object`, malformed tool-call recovery). This is the
  replacement for everything `legacy_provider.rs` still does.
- **Retry/fallback/error classification:** `harness::retry::{RetryPolicy,
  FallbackPolicy, ProviderFailureClass, classify_provider_failure, is_retryable}`
  (`harness/retry/types.rs:28`, `retry/mod.rs:170,396`) — already wired into the
  crate's transport and SSE paths.
- **Routing:** `registry::router::{ModelRouter, WorkloadRoute}` (#54) — adopted
  host-side by #4783; plus `registry::catalog::ModelCatalog` (windows, pricing,
  capability flags, offline snapshot at
  `docs/modules/registry/model-catalog.snapshot.json`).
- **Tools:** `harness::tool::{Tool<State>, ToolSchema}` + schema validation
  (`harness/tool/types.rs:344`, `tool/schema.rs`), `ToolTimeout`, tool-policy
  middleware (`harness/middleware/library/tool_policy.rs`).
- **Subagents/orchestration:** `harness::subagent::{SubAgent, SubAgentSession,
  SubAgentTool}`, `graph::orchestration::{TaskStore, SteeringRegistry,
  OrchestrationTool, InMemoryTaskStore, JsonlTaskStore}`,
  `graph::subagent_node`, `graph::parallel::map_reduce`, and (post-bump)
  `BarrierRelief` fan-in.
- **Embeddings:** provider implementations ported at HEAD (#58 — `openai`,
  `cohere`, `voyage`, `ollama`, `cloud`, rate-limit/retry-after helpers).
- **Observability:** harness + graph journals/status stores, `JsonlSink`,
  `RedactingSink`, Langfuse client with nested run trees (#53).
- **Also present:** summarization (`Summarizer`, `CompressionFailurePolicy`),
  steering, workspace isolation, response cache, cost/usage accounting
  (parses `cached_tokens`), graph checkpointing (SQLite via the `sqlite`
  feature the host enables), and offline `MockModel`/testkit conventions.

~25 public traits are the extension points; the seam already implements 11 of
them (§4.3).

---

## 4. Audit: what remains host-side

### 4.1 `src/openhuman/inference/` — 112 files, ~46.5k LOC

The model-layer inversion (#4727 "Motion B") is scaffolded and mostly cut over,
but the legacy `Provider` stack is still present and **still the default
construction path** in places:

**Still-duplicating cluster (delete in WP-1):**

| File | LOC | State |
| --- | --- | --- |
| `provider/traits.rs` (+`traits_tests.rs`) | 779 + 501 | The `Provider` trait + `ChatMessage`/`ChatRequest`/`ChatResponse`/`ProviderDelta`/`UsageInfo`. Still central; ~10 impls remain. |
| `provider/reliable.rs` (+`reliable_tests.rs`) | 952 + 1,228 | Retry/backoff `Provider` wrapper. Crate `RetryPolicy` covers it; the interactive turn path already stopped wrapping in `ReliableProvider` (`tinyagents/mod.rs:122`). |
| `provider/router.rs` (+`router_tests.rs`) | 310 + 524 | `RouterProvider` hint-table routing. Crate `ModelRouter` adopted by #4783; this is the residual legacy path. |
| `provider/legacy_provider.rs` | 343 | The collapsed remnant of the 15 `compatible*.rs` files (`provider/mod.rs:14` aliases `pub use legacy_provider as compatible;`). Superseded by crate `OpenAiModel` once WP-1 flips the default. |
| `provider/error_classify.rs` + `error_code.rs` | 1,001 + 349 | Split (WP-1): generic retryability/status classification → crate `classify_provider_failure`; openhuman semantics (Sentry demotion, budget messaging, `config_rejection.rs`, `billing_error.rs`) stay as a host classifier layered on crate errors. |
| `provider/crate_provider.rs` | 327 | `CrateBackedProvider` — the **temporary reverse adapter** (crate `ChatModel` → legacy `Provider`). Deleted last, when no consumer needs `Provider`. |

**Crate-native scaffolding already in place (finish in WP-1):**

- `provider/crate_openai.rs` (347) — host `AuthStyle` → crate `OpenAiModel`
  builder; self-described "**Status: scaffolding**" — complete and unit-tested
  but **not yet the factory default**. Flipping it (with per-provider wire
  parity validation) is the gating step.
- `provider/openhuman_backend_model.rs` (325) — managed backend as crate
  `ChatModel` (cut over; coexists with the legacy `openhuman_backend.rs` twin,
  which can go once nothing constructs it).
- `provider/factory.rs` (2,850) — the host↔crate boundary; already returns
  `Arc<dyn ChatModel<()>>` via `create_chat_model*`, but bespoke slugs still
  route "→ a `ProviderModel` over the host provider" (`factory.rs:1458-1511`).
- `model_context.rs` — already falls through to the crate table
  (`context_window_for_model_id`, `model_context.rs:77`); keeps host config
  overrides. Done; keep the thin shim.
- `ops.rs`, `local/ops.rs`, `http/server.rs` already build crate
  `ModelRequest`/`Message` types.

**Seam concerns mis-housed under `provider/` (re-home in WP-1):**
`temperature.rs` (261), `thread_context.rs` (120), `resolved_route.rs` (113),
`auth_error_registry.rs` (166).

**Host-owned, stays (no action):** all of `local/` (~14k — Ollama/LM Studio/
Whisper/Piper runtime + installers + service admin), `voice/` (~2.4k),
`openai_oauth/` (~1.5k), `http/` (0.7k `/v1` server), `provider/ops/` (RPC),
bespoke `Provider`/`ChatModel` impls (`claude_code/` ~2.7k,
`claude_agent_sdk/`, `openai_codex.rs`, `openhuman_backend*.rs`), and the root
host files (`schemas.rs`, `presets.rs`, `model_ids.rs`, `paths.rs`, `parse.rs`,
`sentiment.rs`, `device.rs`).

**Blast radius:** 187 files import `openhuman::inference`; 158 import
`inference::provider` (grown from the old plan's 170/151). Top consumers:
`agent/harness/session/tests.rs` (33 hits), `core/observability.rs` (31),
`tinyagents/mod.rs` (15), `web_chat/web_errors.rs`, `routing/factory.rs`,
`agent/harness/subagent_runner/ops/{graph,runner}.rs`, `voice/`, `flows/ops.rs`,
`config/ops/model.rs`, `channels/context.rs`.

### 4.2 Agent domains

| Domain | Files / LOC | Disposition |
| --- | --- | --- |
| `agent/` | 144 / 66.9k | **Mostly stays** (product brain). 37 files already crate-backed. WP-3 verified that `session/turn/core.rs` is the product turn-preparation shell and `subagent_runner/ops/graph.rs` unconditionally calls `run_turn_via_tinyagents_shared`; the legacy loops and runtime escape hatches were removed before this audit. Heaviest coupling in the tree (config=31, memory=20, event_bus=13 files). |
| `agent_orchestration/` | 64 / 27.9k | Engine already on `tinyagents::graph` (workflow runs, teams, delegation, `map_reduce`); what remains is the product layer (ledgers, RPC). Residual upstream item: detached-subagent `TaskStore` lifecycle — WP-5. 25 files crate-backed. |
| `routing/` | 8 / 2.7k | **Deleted in WP-2.** A repository-wide reference audit found no consumer outside the module; #4783's crate `ModelRouter` already owns the live path, so retaining the nominally host-specific health/provider files would preserve an unreachable parallel stack. |
| `model_council/` + `council_registry/` | 8 / 1.7k | **Deleted after WP-2 audit.** Generic fan-out already used crate `parallel::map_reduce`; upstream dead-code cleanup removed the remaining unreachable product and registry surfaces. |
| `tool_timeout/` | 1 / 316 | **HOST-OWNED.** TinyAgents `harness::tool::ToolTimeout` is declarative per-tool metadata; it has no process-global value to receive OpenHuman config. This module owns UI/config + env precedence and the actual `tokio::time::timeout` deadline used by the host-tool adapter — WP-2 audit closed. |
| `tool_status/` | 3 / 0.7k | **HOST-OWNED.** Its serialized UI/persistence taxonomy, OpenHuman security markers, retry categories, and user-facing remediation copy are product semantics. TinyAgents tool outcomes remain the generic execution input — WP-2 audit closed. |
| `tools/` | 98 / 40.9k | The `Tool` trait + `ToolSpec` mechanics (`traits.rs:255`), `schema.rs`, `policy.rs`, `orchestrator_tools.rs`, `user_filter.rs` are framework-shaped; all of `tools/impl/*` + RPC (`schemas.rs` 985, `ops.rs` 1,407) are product. **Design-gated** (port-plan §2 blockers still apply) — WP-4. Note: `ToolResult`/`ToolContent` are re-exported from `skills::types` (~236 consumer files) — moving those types is the highest-blast-radius single step in the whole migration and must be its own slice. Security coupling: 49 files. |
| `tool_registry/` | 7 / 1.8k | Read-only cross-surface discovery + RPC. Stays. |
| `agent_registry/`, `agent_experience/`, `agent_memory/`, `agent_tool_policy/`, `agentbox/`, `orchestration/` | — | All host/product (definitions-as-data, RPC controllers, marketplace HTTP, remote-brain client — `orchestration/` talks to the hosted backend, not the local crate). Stay. `agent_tool_policy` overlaps crate `tool_policy` middleware mechanically but encodes host channel-permission policy — stays, mechanism may thin post-WP-4. |

### 4.3 The seam — `src/openhuman/agent/tinyagents/` (25 files, ~17.1k LOC)

The seam is healthy: 23/25 files use the crate; it implements `Middleware`
(13×), `ToolMiddleware` (4×), `ModelMiddleware` (3×), `ChatModel` (2×),
`Summarizer`, `Tool`, `EventListener` (2×), `PayloadSummarizer`,
`HarnessStatusStore`, `EmbeddingModel`, and `GraphEventSink`. Post-migration it
**shrinks but remains host-owned**. Specific shrink targets:

- **`model.rs`: `ProviderModel` is deleted.** The former `convert.rs` was split:
  tool-schema conversion remains as the 34-line WP-4 seam, while durable-message
  conversion moved to `agent/message_convert.rs`. The execution audit found that
  `ChatMessage` is OpenHuman's versioned JSONL/thread persistence record (including
  message ids and product metadata), not a duplicate provider request type;
  replacing it directly with the crate enum would change existing on-disk data.
- **`middleware.rs` (4,702):** the WP-5 audit records all 18 concrete types
  found at audit time (the tool-exposure shadow was added after the 17-type
  snapshot) in `docs/tinyagents-drift-ledger.md`. `SchemaGuard` is now deleted
  in favor of TinyAgents 2.1 `InvalidArgsPolicy::ReturnToolError`, leaving 17.
  `ArgRecovery` overlaps crate #45/#71 normalization. `RepeatProgress` now uses
  the merged #72 crate tracker; only the thin OpenHuman signature/exemption and
  halt-projection adapter remains. Upstream the generic ones; host-policy ones
  (ApprovalSecurity, CredentialScrub, CliRpcOnly, MemoryProtocol, CostBudget)
  stay — WP-5.
- `routes.rs` `UsageCarry`/`FallbackObserver` thin out as usage/fallback become
  fully crate-native.

### 4.4 Broken/dangling references (resolved in WP-0)

WP-0 retargeted SDK-gap references to
`vendor/tinyagents/docs/sdk-gaps.md`, made the provider schema
`src/openhuman/config/schema/cloud_providers.rs` authoritative, created
`docs/tinyagents-full-migration-plan/99-deletion-ledger.md`, replaced the
phantom numbered-plan links with this plan's WP-5/C4 sections, and retargeted
the TinyCortex analogy to this current audit. The three historical plans now
carry explicit superseded banners.

### 4.5 Test inventory (what moves, dies, or stays)

- **Dies with WP-1** (bound to the legacy `Provider` stack):
  `provider/traits_tests.rs` (501), `reliable_tests.rs` (1,228),
  `router_tests.rs` (524) — these define 10+ mock `impl Provider` doubles.
  `factory_tests.rs` (3,149) shrinks and retargets to `ChatModel` construction.
  In `tests/raw_coverage/`: the three `inference_compatible_*` files (~2.2k
  LOC) are pinned to the deleted wire client — retire or rewrite the still-
  meaningful assertions against `OpenAiModel` via wiremock.
- **Retargets:** `tests/inference_provider_e2e.rs` (568, wiremock-based —
  OpenAI-compat chat/streaming, Anthropic auth headers, temperature
  suppression, Ollama `/v1`) — keep the scenarios, point them at the crate-
  native factory path; these become the **wire-parity gate** for flipping the
  default.
- **Moves upstream (as crate tests, offline `MockModel` convention):** parity
  coverage for anything upstreamed in WP-2/WP-5 (routing policy/quality,
  council graph, middleware behaviors). The crate's CI is
  fmt → clippy (`-D warnings`, both feature sets) → build → `cargo test` +
  `--all-features`, no network.
- **Stays host-side:** seam tests (`tinyagents/tests.rs` 799 + 19 inline),
  `tests/agent_harness_e2e.rs` (130 KB), `tests/json_rpc_e2e.rs` (576 KB),
  `agentbox_e2e`, `orchestration_*`, `embeddings_rpc_e2e`,
  `ollama_lifecycle_e2e`, `openai_oauth/flow_tests.rs`, all `local/*_tests`,
  and the `raw_coverage` family except the compatible trio.
- **Runner impact:** `scripts/test-rust-with-mock.sh:52` had a stale comment
  advertising removed `OPENHUMAN_AGENT_GRAPH_*` escape hatches. WP-3 removes
  that documentation; the runner behavior itself was already crate-only.

---

## 5. The plan — six work packages

Ordering rationale: WP-0 is pure hygiene and unblocks honest review of
everything else; WP-1 is the largest deletion and already 80% scaffolded;
WP-2/WP-3 are independent of each other and of WP-1's tail; WP-4 is the only
design-gated package; WP-5 is opportunistic shrink; WP-6 is the exit gate.
Every WP follows the repo's standing rules: failing-before/passing-after
regression tests, small validated commits on a feature branch per submodule,
drift-ledger rows updated before deletions (the ledger's gate rule stays in
force).

### WP-0 — Version + docs housekeeping (no behavior change)

1. Bump `vendor/tinyagents` submodule to tag `v2.1.0` (fast-forward; picks up
   `BarrierRelief` #62 and the bytes bump). Regenerate root and
   `app/src-tauri` lockfiles. `cargo check` both worlds +
   the slim disabled build (`--no-default-features --features
   tokenjuice-treesitter`) per repo convention.
2. Update `tinyagents-drift-ledger.md` Anchors (pin row → v2.1.0; add rows
   closing out #4780/#4782/#4783/#4784 which the ledger still lists as
   pending/deferred; mark the Phase-3 "compatible*.rs remains" text CLOSED).
3. Stamp superseded-by banners on `tinyagents-port-plan.md`,
   `tinyagents-inference-migration-plan.md`,
   `tinyagents-phase3-router-registry-design.md` pointing here (keep them —
   their disposition tables and gap lists are still the reference detail).
4. Fix the §4.4 broken references, create the deletion ledger, and replace the
   phantom numbered-plan references with real WP-5/C4 anchors.
5. Remove the stale fork-source patch after verifying that no dependency uses
   it. Align TinyCortex's nested path dependency through a crates.io `2.1`
   declaration plus its repo-local development patch, so the host's canonical
   path patch produces one trait identity (TinyCortex #121).

**Exit:** `Cargo.toml`, submodule, lockfiles, and ledger all agree on one
version; zero dangling `docs/tinyagents*` references (`grep -rn` clean).

### WP-1 — Finish the model-layer cutover; delete the legacy `Provider` stack

The continuation of #4727 Motion B / drift-ledger P1-8/P1-9. Slices:

1. **Flip the factory default** to `crate_openai.rs` construction for every
   OpenAI-compatible slug. Gate: the retargeted `inference_provider_e2e.rs`
   wire-parity suite (§4.5) green against the crate path, per provider family
   (managed, BYOK, ollama/lm-studio local, codex/responses).
2. **Route bespoke providers as `ChatModel`s** — `claude_code`,
   `claude_agent_sdk`, `openai_codex`, `openhuman_backend` get direct
   `ChatModel` impls (the pattern `openhuman_backend_model.rs` already
   proves), removing the last `factory.rs:1458-1511` `ProviderModel` wraps.
   Delete the legacy `openhuman_backend.rs` twin when nothing constructs it.
3. **Error-taxonomy split:** generic classification call sites move to crate
   `classify_provider_failure`/`ProviderError`; `error_classify.rs` shrinks to
   the openhuman-semantic layer (`config_rejection`, `billing_error`, Sentry
   demotion). Upstream any status codes the crate misclassifies (drift-ledger
   rows, then crate PR + version bump).
4. **Delete, in dependency order:** `router.rs` → `reliable.rs` →
   `legacy_provider.rs` (+ the `as compatible` alias in `provider/mod.rs:14`)
   → `provider/traits.rs` → seam `ProviderModel`/`MaxTokensModel`
   (`tinyagents/model.rs`) → the reverse adapter `crate_provider.rs` last.
   Re-home the durable transcript adapter beside the agent persistence DTO;
   retain only tool-schema conversion in the seam until WP-4. Each deletion is its own commit with its test
   fallout (§4.5) handled in the same commit.
5. **Consumer sweep:** migrate the 187 importing files to crate types
   (`Message`, `ModelRequest`, `Usage`, `TinyAgentsError`). Mechanical for
   most; the hot spots are `core/observability.rs`, `web_chat/web_errors.rs`,
   `voice/`, `flows/ops.rs`, `config/ops/model.rs`.
6. **Re-home** `temperature.rs`, `thread_context.rs`, `resolved_route.rs`,
   `auth_error_registry.rs` out of `provider/` into the seam (or the factory
   module) so `provider/` ends as: factory + bespoke models + ops/RPC.

**Exit criteria:** no `impl Provider` anywhere; `grep -rn "ProviderModel"
src/` empty; `inference/provider/` ≤ ~8k LOC (from ~15.9k + tests);
`inference_provider_e2e` + `agent_harness_e2e` + full
`scripts/test-rust-with-mock.sh` green. Estimated net deletion: **~9–10k LOC**
(code + dead tests).

### WP-2 — Consolidate parallel implementations onto crate primitives

Independent, individually shippable slices:

1. **`routing/` → `registry::router` — complete by deletion.** The execution
   audit found the entire module self-contained: no production or test caller
   outside `routing/` constructed its provider or consumed its health signals.
   #4783's crate `ModelRouter` already owns live workload routing, so WP-2
   removed the unreachable parallel stack and its self-tests.
2. **`tool_timeout/` — HOST-OWNED (audit closed).** The similarly named crate
   `ToolTimeout` is per-tool policy metadata, not a global timeout store or
   executor. OpenHuman must retain config/env precedence, live UI updates,
   seconds clamping, grace handling, and the adapter's hard deadline. The seam
   already projects host policy into crate `ToolRuntime`; there is no duplicate
   crate implementation to delete.
3. **`model_council/` — complete by deletion.** Generic member fan-out already
   ran through `tinyagents::graph::parallel::map_reduce`; upstream dead-code
   cleanup removed the remaining unreachable council and registry product
   surfaces.
4. **`tool_status/` — HOST-OWNED (audit closed).** The classifier is tied to
   OpenHuman security markers, serialized thread/UI types, retry categories,
   localized-copy keys, and remediation language. TinyAgents owns generic tool
   outcomes; mapping those outcomes into product status remains in the host.

**Exit: complete.** `routing/`, `model_council/`, and `council_registry/` are
deleted; timeout and status are explicitly HOST-OWNED. Ledger rows record the
audit evidence.

### WP-3 — Retire the legacy turn engine

**Audit result: complete before this plan was written.** The filenames cited by
the initial audit survived, but their legacy engines did not:

1. `session/turn/core.rs` performs OpenHuman turn preparation and calls the
   TinyAgents session path; it contains no `run_turn_engine` definition.
2. `subagent_runner/ops/graph.rs` documents the removed `run_inner_loop` /
   `run_turn_engine` and unconditionally calls `run_turn_via_tinyagents_shared`.
3. No runtime or test code reads `OPENHUMAN_AGENT_GRAPH_*`; only this plan and a
   stale test-runner comment mentioned those names. The stale comment is now
   removed.

**Exit satisfied:** one TinyAgents turn engine; source and scripts contain no
`OPENHUMAN_AGENT_GRAPH_*` reference. Historical/parity comments mentioning the
former engine remain intentionally as provenance, not executable branches.

### WP-4 — Tool-model reconciliation (design-gated; do not start on autopilot)

The openhuman `Tool` trait (`tools/traits.rs:255`) vs crate `Tool<State>`
unification. The port-plan §2 design blockers still stand, plus two audited
constraints:

- `ToolResult`/`ToolContent` live in `skills::types` as the deliberately
  ungated type carve-out consumed by ~236 files (MCP, runtime_node, every Tool
  impl). Any move must preserve that carve-out property (inert, dep-free,
  compiled in all feature combinations) — either the types move **into the
  crate** and `skills::types` re-exports them, or they stay and the crate
  bridge continues via `SharedToolAdapter`.
- `tools/` has the tree's heaviest security coupling (49 files) — execution
  gating (`classify_command`, approval, sandbox) must stay host-side
  regardless of where the trait lives.

Recommended shape: a short design doc (successor to the port-plan §2 list)
deciding (a) trait unification vs permanent adapter, (b) the
`ToolResult`/`ToolContent` home, (c) `ToolScope::AgentOnly` gating (deferred
"Phase 2" per `traits.rs:14-19`) — then slices. Until then, the
`SharedToolAdapter` seam is cheap and correct; **do not delete it
prematurely.**

The proposed decision is recorded in
[`tinyagents-tool-model-decision-2026-07-23.md`](tinyagents-tool-model-decision-2026-07-23.md).
It selects a permanent ownership-boundary adapter, keeps the host's structured
result types in the always-compiled carve-out while preserving them through
TinyAgents `raw`, and defines the required scope-enforcement matrix. WP-4 code
changes remain gated on explicit approval of that proposal.

### WP-5 — Seam shrink + orchestration lifecycle upstreaming

1. Middleware audit (§4.3): per-middleware ledger rows; `SchemaGuard` is deleted
   through the existing crate invalid-args policy. TinyAgents #72 is merged and
   `RepeatProgress` now delegates generic streak accounting to the crate
   `SuccessfulRepeatTracker`; the host duplicate guard and thresholds are
   deleted, while the thin product adapter remains. `ArgRecovery` still awaits
   the #71 crate equivalent; keep host-policy middlewares.
2. Detached-subagent lifecycle: move the generic detached-run registry
   mechanics (`agent_orchestration/running_subagents.rs`,
   `subagent_control.rs`) onto crate `TaskStore`/`SteeringRegistry` fully;
   host keeps ledgers + RPC. This restores the lifecycle consolidation intent
   recorded by the earlier audit without relying on a phantom design doc.
   **Cutover implemented:** TinyAgents PR
   [#75](https://github.com/tinyhumansai/tinyagents/pull/75) adds the generic
   ownership-aware `DetachedTaskRegistry`; OpenHuman now delegates snapshots,
   waits, steering lookup, cancellation/abort, terminal sweeping, and lock-error
   handling to it. The host retains durable `TaskStore` projection, product
   metadata, RPC, and the `RunQueue` compatibility fallback. All 17 focused
   `running_subagents` tests pass. TinyAgents #75 merged as `d548657`, and the
   canonical vendored pointer `4358efe` includes it, closing the generic
   lifecycle cutover.
3. Finish the C4 journal-progress-parity plan (S2–S6): journal-backed progress
   projection, then delete `agent/progress_tracing.rs` (1,272) +
   `progress_tracing/langfuse.rs` (825) in favor of crate observability —
   respecting the recorded S2a replayability blocker.

### WP-6 — Exit gate

- Full suite: `scripts/test-rust-with-mock.sh` (all targets incl.
  `raw_coverage_all`), `cargo test --all-features` in `vendor/tinyagents`,
  slim disabled build + `cargo test --lib --no-default-features --features
  tokenjuice-treesitter core::` (repo standing rule), `pnpm rust:check`.
- Deletion ledger totals reconciled against the port-plan's projection
  (~30k deleted / ~12–15k upstreamed).
- Docs: `inference/README.md`, `gitbooks/developing/architecture/
  {agent-harness,orchestration,flows-on-tinyagents}.md` updated to the
  post-cutover reality; drift ledger phases marked CLOSED; this plan stamped
  done.

---

## 6. Risks and standing gotchas

- **Wire parity is the whole risk in WP-1.** The legacy client accumulated
  provider quirks for years; the crate's #44–#57 wave was built to match them,
  but the flip must be gated on the wiremock parity suite per provider family,
  not on unit tests. Keep `CrateBackedProvider` as the rollback seam until the
  parity suite has soaked on `main`.
- **GPL/crates.io boundary:** tinyagents is publicly redistributed — only
  genuinely generic code moves down; no product policy, backend phrasing as
  API, or key material (port-plan rule, still in force).
- **Usage fidelity (old gap G1):** verify crate `Usage` carries the
  USD/cached-token fields the cost tracker needs before deleting
  `UsageInfo`; `routes.rs` `UsageCarry` is the current compensator.
- **`RUST_MIN_STACK=16777216`** in the mock runner exists because of the
  subagent runner's large futures — deep-graph changes in WP-3/WP-5 can
  resurface stack overflows on Apple Silicon (see
  `composio_list_tools_stack_overflow_regression` test).
- **Whisper/GGML on Apple Silicon:** any `cargo` validation of the root crate
  locally needs `GGML_NATIVE=OFF` (unrelated to this migration but will bite
  every WP's validation loop).
- **Two Cargo worlds:** every vendor bump must update both the root and
  `app/src-tauri` lockfiles (until the workspace convergence tracked in
  #3877).

---

## 7. Quick reference — disposition of every audited module

| Module | Verdict |
| --- | --- |
| `inference/provider/{traits,reliable,router,legacy_provider,crate_provider}.rs` | DELETE (WP-1) |
| `inference/provider/{error_classify,error_code}.rs` | SPLIT: generic → crate; host semantics stay (WP-1) |
| `inference/provider/{crate_openai,openhuman_backend_model,factory}.rs` | KEEP — becomes the host↔crate boundary (WP-1 finishes) |
| `inference/provider/{temperature,thread_context,resolved_route,auth_error_registry}.rs` | RE-HOME to seam (WP-1) |
| `inference/{local,voice,http,openai_oauth}/`, `provider/ops/`, bespoke providers, root host files | STAYS |
| `tinyagents/model.rs` (`ProviderModel`) | DELETED (WP-1) |
| `tinyagents/convert.rs` message layer | RE-HOMED to `agent/message_convert.rs`; durable JSONL/thread compatibility is product-owned. Tool-schema conversion remains for WP-4. |
| `tinyagents/middleware.rs` generic middlewares | UPSTREAM case-by-case (WP-5) |
| `tinyagents/` remainder (seam) | STAYS, shrinks |
| `agent/` legacy `run_turn_engine` + escape hatches | ALREADY DELETED; WP-3 corrected the stale audit and runner documentation |
| `agent/` remainder, `agent_registry/`, `agent_experience/`, `agent_memory/`, `agent_tool_policy/`, `agentbox/`, `orchestration/`, `tool_registry/` | STAYS (product/host) |
| `routing/` | DELETED; #4783 crate router already owned the only live path (WP-2) |
| `tool_timeout/` | HOST-OWNED: config/env state + hard-deadline enforcement; crate timeout is metadata only (WP-2 closed) |
| `tool_status/` classification | HOST-OWNED: OpenHuman UI/security/recovery taxonomy (WP-2 closed) |
| `model_council/`, `council_registry/` | DELETED after crate-backed fan-out audit and upstream dead-code cleanup (WP-2 closed) |
| `tools/` trait mechanics | DESIGN-GATED (WP-4) |
| `tools/impl/*`, all `schemas.rs` RPC controllers | STAYS |
| `agent_orchestration/` detached-run mechanics | UPSTREAM to `TaskStore` (WP-5) |
| `agent/progress_tracing*` | DELETE after C4 S2–S6 (WP-5) |
