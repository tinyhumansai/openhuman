# TinyAgents Port — Plan & Audit (inference / tools / agent_orchestration)

**Status:** Phase 1 partial host cutover — TinyAgents `v1.7.1` is tagged and
the host now pins it; `SchemaCleanr`, the generic model-context table, and the
context-preserving `invoke_stream` path are crate-backed.
**Anchor precedent:** the TinyAgents harness migration (#4249 / #4399 / #4473) and the TinyCortex memory migration plan (`docs/tinycortex-memory-migration-plan.md`).
**Target:** move the genuinely framework-shaped parts of `src/openhuman/inference/`, `src/openhuman/tools/`, and `src/openhuman/agent_orchestration/` down into the `tinyagents` crate (vendored git submodule at **`vendor/tinyagents`**, `https://github.com/tinyhumansai/tinyagents`), and delete the in-tree duplicates in favor of crate primitives.

---

## 0. Ground truth (as audited)

### 0.1 Sizes and shape

| Host module | LOC (approx) | Role |
| --- | --- | --- |
| `src/openhuman/inference/` | ~53,000 (121 files) | Provider trait + OpenAI-compatible wire client, factory/routing, local runtime (Ollama/LM Studio/Whisper/Piper), voice, OAuth, RPC surface |
| `src/openhuman/tools/` | ~38,500 | `Tool` trait + metadata model, schema cleaning, generated-tool admission, ~200-tool registry assembly, `impl/` families (filesystem, system, browser, computer, network, presentation), RPC surface |
| `src/openhuman/agent_orchestration/` | ~25,800 | Product control plane over sub-agents: in-memory session, detached-run registry, workflow runs, agent teams, command center, worktree isolation, RPC/tools surface |
| `src/openhuman/tinyagents/` (the seam) | ~15,200 (25 files) | Adapters implementing tinyagents traits over openhuman services — where all ported code plugs back in |

**TinyAgents** (sibling repo, v1.7.1, edition 2024, GPL-3.0-only, published to crates.io) already provides: `harness/` (`ChatModel`, `Tool<State>`, agent loop, middleware, retry/fallback, usage/cost, cache, embeddings + vector store, summarization, steering, `SubAgent`/`SubAgentSession`/`SubAgentTool`, observability journals, `WorkspaceIsolation`, testkit), `graph/` (durable typed graphs, checkpointers incl. SQLite, recursion policy, `map_reduce`, `orchestration::TaskStore`/`SteeringRegistry`, topology export), `registry/`, and the `.rag`/`.ragsh` languages. ~30 public traits are the extension points.

**License:** both repos are GPL-3.0 — moving code down is license-clean. But tinyagents is publicly redistributed on crates.io, so only genuinely generic code moves (no product policy, backend phrases as load-bearing API, or keys).

### 0.2 The single most important audit finding

**Most of the "bulky" code is not portable — it is a parallel implementation of things tinyagents already ships.** The heavy clusters duplicate crate primitives rather than extend them:

| openhuman (in-tree) | tinyagents (already shipped) |
| --- | --- |
| `inference/provider/traits.rs` (`Provider`, `ChatMessage`, `ChatResponse`, `UsageInfo`) | `harness/model/` (`ChatModel`, `ModelRequest/Response`), `harness/usage/` (field-for-field) |
| `inference/provider/compatible*.rs` (~6.6k + 4.9k tests) | `harness/providers/openai/` (same 9-provider OpenAI-compat fan-out) |
| `inference/provider/reliable.rs` (retry wrapper) | `harness/retry/` + `middleware/library/resilience.rs` — `reliable.rs`'s own module doc says it is "slated for removal once the tinyagents crate owns retry/fallback" and pins retry to 1 attempt |
| `tools/traits.rs` `Tool` trait / `ToolSpec` / flat-Vec registry | `harness/tool/` `Tool<State>` / `ToolSchema` / deduping `ToolRegistry` |
| `agent_orchestration/ops.rs` `AgentOrchestrationSession` + `running_subagents.rs` | `graph/orchestration/` `TaskStore` + `SteeringRegistry` + `harness/subagent/` `SubAgentSession` |
| `inference/provider/router.rs` tier routing | `harness/middleware` fallback + `registry/` `ModelCatalog` |

So the correct plan has **three motions**, not one:

1. **Consolidate (delete in favor of the crate)** — the duplication above. This is where nearly all of the LOC reduction comes from.
2. **True ports (net-new upstream)** — a short list of clean, generic pieces tinyagents lacks (§2).
3. **Stays in openhuman** — RPC/controllers, config/credentials/policy glue, local runtime + voice, product tools, UI surfaces (§3).

### 0.3 Version status

**Phase 0 closed:** the host branch first aligned `vendor/tinyagents` to
`v1.6.0` and closed the baseline seam fallout: `ToolCompleted` outcome
projection, SHA-256 prompt-cache fingerprinting, and the redaction-layer audit.

**Phase 1 partial cutover:** TinyAgents `v1.7.1` is tagged at
`3e81e493`, and the host now pins both lockfiles plus `vendor/tinyagents` to
that release. `SchemaCleanr` is crate-backed through the stable OpenHuman
import path; `model_context.rs` delegates generic raw-model hints to
`tinyagents::harness::model::context_window_for_model_id` after checking
OpenHuman tier aliases and the cost catalog; and the observed run path now
drives `invoke_stream_in_context`, whose `Send` stream fix shipped in
[tinyagents#28](https://github.com/tinyhumansai/tinyagents/pull/28).

### 0.4 Contribution workflow (same convention as tinycortex)

Engine changes are made **inside `vendor/tinyagents`**, committed on a branch there, PR'd against `tinyhumansai/tinyagents`, released, then the host bumps the submodule SHA + version pin in a standalone `chore(vendor): bump tinyagents — <summary> (tinyagents#<PR>)` commit. Host PRs never contain engine source edits. tinyagents house rules apply to upstreamed code: `types.rs`/`test.rs`/`mod.rs` module discipline, centralized `lib.rs` re-exports, ≥80% coverage, tiny dependency tree (no new default deps; anything heavy goes behind a cargo feature), offline tests via `MockModel` (network tests are `live_*` and key-gated).

---

## 1. Target ownership split

### 1.1 Moves to / consolidates into TinyAgents

| Host code | Motion | tinyagents destination |
| --- | --- | --- |
| `inference/provider/error_classify.rs` (777 L, 35 tests, extracted for this purpose in #4249) | port | `harness/retry/` classification (or `harness/providers/` error module) |
| `inference/model_context.rs` pattern table (minus OH tier/cost-catalog arms) | port | `harness/model/` `ModelProfile`/context metadata |
| `inference/parse.rs` (`sanitize_inline_completion`, zero deps) | port (low priority) | text post-process util |
| `inference/provider/compatible*.rs` + `reliable.rs` + `router.rs` | **delete** after cutover to `harness/providers/openai` + `retry`/`resilience` + registry routes | n/a (crate already has it) |
| `tools/schema.rs` `SchemaCleanr` (+ `schema_tests.rs`, zero deps) | port | provider adapters (per-provider JSON-Schema cleaning — genuine gap) |
| `tools/traits.rs` display/metadata morsels: `humanize_tool_name`, `context_detail_from_args`, `display_label`/`display_detail`, `ToolTimeout` semantics | port (additive) | `harness/tool/` (`ToolRuntime`/policy metadata) |
| `tools/generated.rs` admission/provenance model | align (don't duplicate) | `registry/capability` + `registry/component` |
| `tools/impl/filesystem/` (file_read/write/edit_file/apply_patch/grep/glob/list_files/read_diff) | port (behind a `tools` feature) | new `harness/tools/` builtin family, `Tool<State>`-native |
| `tools/impl/system/{current_time,resolve_time}.rs` | port (first movers — nearly pure) | same builtin family |
| `tools/impl/network/{http_request,web_fetch,curl,url_guard}.rs` (SSRF guard is 842 L of largely pure logic) | port | same builtin family |
| `tools/impl/system/{shell,node_exec,npm_exec}.rs` | port later (multi-seam: SecurityPolicy, NodeBootstrap, sandbox, tool_timeout) | same builtin family, gated on §4 blockers |
| `agent_orchestration/worktree.rs` (617 L — already an impl of tinyagents' `WorkspaceIsolation`; one `publish_global` seam) | port | `harness/workspace/` git-worktree isolation provider |
| `agent_orchestration/ops.rs` (`AgentOrchestrationSession`) + `running_subagents.rs` (1.9k L) | **delete** in favor of `graph::orchestration::TaskStore` + `SteeringRegistry` + `SubAgentSession`; route the live control path through the crate (today only re-exported, per seam `orchestration.rs:23-33`) | n/a |
| `agent_orchestration/types.rs` `AgentStatus` vocabulary | reconcile into `OrchestrationTaskStatus` (two parallel status enums today) | `graph/orchestration/types` |
| `agent_orchestration/workflow_runs/` phase-DAG validation + `agent_teams/` dependency-DAG/atomic-claim/quality-gate logic | evaluate upstreaming the *validation/scheduling slices* as graph extensions; durability (`session_db::run_ledger`) and RPC stay host-side | `graph/` |
| Reasoning-content channel (historically smuggled via `ContentBlock::ProviderExtension`; host now writes `ContentBlock::Thinking` for new messages and only reads legacy `ProviderExtension` from persisted transcripts — seam `convert.rs`) | **largely done** — the crate's first-class typed reasoning channel (`ContentBlock::Thinking`) is live per Phase 1.4 / ledger P1-5 | `harness/message/` |

### 1.2 Stays in OpenHuman (product policy, I/O, surfaces)

- **All RPC surfaces**: `inference/{ops,schemas}.rs`, `provider/ops*`, `tools/schemas.rs`, `agent_orchestration/*_schemas.rs`, `subagent_control.rs`, `command_center/`, `worktree_schemas.rs`. JSON-RPC method names and payload shapes must not change.
- **Provider resolution & auth**: `provider/factory.rs` (provider-string grammar), `openhuman_backend.rs`, `openai_codex.rs`, `claude_code/`, `claude_agent_sdk/`, `openai_oauth/`, `thread_context.rs`, backend billing-envelope parsing (`openhuman.usage.*` / `openhuman.billing.*`).
- **Local runtime & voice** (~17k L): all of `inference/local/` (Ollama/LM Studio admin, Whisper/Piper install + engines), `voice/`, `device.rs`+`presets.rs`+`model_ids.rs`+`paths.rs` (device→tier product policy), `sentiment.rs`, `http/` (OpenAI-compat serving surface). tinyagents has no local-runtime provisioning and should not grow one now. (`device.rs` is technically portable — zero openhuman deps — but only valuable if tinyagents ever grows local provisioning; skip.)
- **Tool product surface**: `tools/ops.rs` registry assembly (~200 registrations over ~50 domains), `user_filter.rs` (UI-toggle families), `orchestrator_tools.rs`, `local_cli.rs`, `impl/computer/` (macOS CGEvent/AX), `impl/presentation/`, network app integrations (polymarket, gitbooks, gmail_unsubscribe, mcp_setup).
- **Orchestration product plane**: all `agent_orchestration/tools/*` (openhuman `Tool` impls re-pointed at crate primitives), `subagent_events.rs` + `run_ledger_finalize.rs` (event-bus bridges), `background_{completions,delivery}.rs` (chat idle-delivery UX), `pairing*` (tiny.place consent), `parent_context/` (DI bootstrap), `session_db::run_ledger` durability.
- **The seam itself** (`src/openhuman/tinyagents/`) — it shrinks as duplication is deleted, but the `ChatModel`/`Tool`/middleware/journal adapters remain openhuman's integration layer.

---

## 2. Blockers to resolve before tool ports (design decisions)

These are API mismatches the current seam bridges **lossily**; porting tools natively forces a decision on each:

1. **`ToolResult` shape** — openhuman uses MCP-style content blocks + `markdown_formatted`; tinyagents uses flat string `content` (the seam always sets `raw: None`, discarding structure). Proposal: add an optional structured-content field (or `raw` population convention) to tinyagents' `ToolResult` so ported tools stop losing block structure.
2. **`PermissionLevel` (5 ordered levels, with per-call `*_with_args` overrides) vs `ToolSideEffects` booleans** — today `Write`/`Execute`/`Dangerous` all collapse to `writes_files` + `WorkspaceAccess::Any`, and per-call gating is dropped. Proposal: extend `ToolPolicy` with an ordered permission level and an optional per-call classifier hook, or accept the boolean model and encode levels host-side only (document the loss).
3. **`SecurityPolicy` is the universal seam** across all of `impl/` (path/command/host gating). tinyagents models the same concept as `ToolAccess`/`SandboxMode`/`WorkspaceDescriptor`; the `security_for_tool_context` shims in `impl/{filesystem,system}/mod.rs` already bridge workspace roots. Ported tools must depend **only** on the crate-side abstractions; openhuman injects its `SecurityPolicy` behind them. This likely means growing `ToolAccess` (e.g. trusted roots, command classification hook) — design this once, before the first filesystem tool moves.
4. **`file_state` edit-tracking** (filesystem family) — either port a minimal read-before-write tracker into the crate tools or leave tracking host-side via middleware.
5. **Status vocabulary** — `AgentStatus` (Pending/Running/Waiting/Completed/Failed/Cancelled/Closed) vs `OrchestrationTaskStatus` (…/Awaiting/Timeout). Pick the crate vocabulary, map host wire compat in the RPC layer.

---

## 3. Step-by-step plan

Each phase is a coherent PR set: tinyagents PR(s) → release/tag → host `chore(vendor): bump` → host cutover PR. Every phase ends green on both CI lanes (tinyagents `ci.yml`; host fast lane + release lane).

### Phase 0 — Version alignment & baseline (no behavior change)

1. **Done:** bump `vendor/tinyagents` to **v1.6.0** and the host requirement `1.5.0` → `1.6.0` (both root and `app/src-tauri` manifests; keep the `[patch.crates-io]` path entries).
2. **Done:** fix seam fallout from the bump: SHA-256 prompt fingerprint vs the seam's KV-cache drift guard (`tinyagents/middleware.rs`), idempotent `RedactionMiddleware` vs `journal.rs` double-redaction, adopt `ToolCompleted` outcome fields in `observability.rs` (retire `ToolFailureMap` reconstruction). The context-preserving `invoke_stream` host path is now closed by the `Send` stream follow-up released in TinyAgents `v1.7.1`.
3. Record a baseline: LOC per module, test counts, and the duplication map (§0.2) as the drift ledger for this migration (mirroring `docs/tinycortex-drift-ledger.md`).
4. **Exit:** host builds and full release lane passes on tinyagents 1.6.0 with zero in-tree deletions yet. The host has since moved on to the Phase 1 `v1.7.1` partial cutover.

### Phase 1 — Quick wins upstream (small tinyagents PRs, no host deletions)

Land these as separate small PRs against tinyagents (each with its ported tests re-expressed in crate conventions):

1. `SchemaCleanr` (+ its 15-test suite) → provider-layer schema cleaning. Zero-dep, highest value/effort ratio. **Status:** released in TinyAgents `v1.7.0`; host `tools/schema.rs` re-exports the crate implementation so existing import paths keep working.
2. `error_classify.rs` classifiers → retry/HTTP-error classification (strip openhuman-backend phrase matches into a host-side extension table; keep the generic HTTP/status logic). **Status:** released in TinyAgents `v1.7.0`; host retry/failure call-site swaps remain pending because OpenHuman-specific backend phrase and billing-envelope rules must stay host-side.
3. `model_context.rs` pattern-match table (substring-vs-segment matching, incl. the o1/o3 regression guard) → `ModelProfile` context resolution. OH tier aliases and cost-catalog arms stay host-side. **Status:** released in TinyAgents `v1.7.0`; host now delegates generic fallback lookup to the crate after checking tiers and the OpenHuman cost catalog.
4. First-class **reasoning channel** on `AssistantMessage` → delete the `ProviderExtension` smuggling in seam `convert.rs`. **Status:** closed for new messages: TinyAgents `v1.6.0` provides typed thinking blocks, and the host now writes `reasoning_content` to `ContentBlock::Thinking` while retaining legacy `ProviderExtension` reads.
5. Git-worktree `WorkspaceIsolation` provider from `agent_orchestration/worktree.rs` (+ `worktree_tests.rs`, which exercises real git repos — fits the crate's offline test policy). The single `publish_global` event becomes a host-side wrapper. **Status:** released in TinyAgents `v1.7.0`; OpenHuman's wrapper stays for event-bus emissions, `OutsideWorkspace`, and host policy mapping until a safer adapter pass.
6. Tool display metadata (`humanize_tool_name`, `display_label`/`display_detail`, `ToolTimeout` semantics) → `harness/tool/`. **Status:** released in TinyAgents `v1.7.0`; host adjusted `ToolRuntime.timeout`, but the OpenHuman `Tool` trait still owns richer legacy metadata until Phase 2 reconciles the tool model.
7. `current_time` / `resolve_time` as the first two builtin tools (pilot for the `tools` feature layout). **Status:** released in TinyAgents `v1.7.0` behind the optional `tools` feature; host wrappers stay until the Phase 2 tool model reconciliation adopts crate builtins.
8. **Exit:** TinyAgents `v1.7.1` tagged and host bumped. `SchemaCleanr`, generic model-context lookup, reasoning, and the context-preserving observed stream path are crate-backed; worktree/time/error-classification/display APIs are available but host deletion waits for their adapter/model reconciliation steps.

### Phase 2 — Tool model reconciliation + builtin tool families

1. Resolve §2 blockers 1–4 as a tinyagents design PR (ToolResult structure, permission model, `ToolAccess` extension, edit tracking).
2. Port `impl/filesystem/` (8 generic tools) natively onto `Tool<State>` behind a `tools` cargo feature, depending only on `ToolAccess`/`WorkspaceDescriptor`. Port inline tests + `git_operations`/`run_tests` siblings where generic.
3. Port `impl/network/{http_request,web_fetch,curl,url_guard}` (SSRF guard first — it is mostly pure).
4. Host cutover: `tools/ops.rs` registers the crate builtins (wrapped with openhuman `SecurityPolicy` injected behind `ToolAccess`); delete the in-tree implementations; `user_filter.rs` table re-points at crate tool names (watch the `web_search`→`"web_search_tool"` name-drift risk).
5. Defer `shell`/`node_exec`/`npm_exec` to a follow-up within this phase — they need the command-classification hook (`classify_command`/`gate_decision` stays host-side; the crate exposes the hook).
6. `generated.rs`: re-target `GeneratedTool` onto the crate `Tool<State>` + align admission/provenance with `registry/capability` instead of a parallel mechanism.
7. **Exit:** filesystem + network + time tools live in tinyagents with their tests; host `impl/` shrinks by ~10k L; `tool_policy_from_openhuman_tool` lossy mapping replaced by the reconciled model.

### Phase 3 — Provider consolidation (deletion, not porting)

The endgame of #4249's Workstream 11:

1. Cut the live chat path over from `Provider::chat`/`compatible*.rs` to `harness/providers/openai` via the seam's `ProviderModel` — provider-by-provider, behind the existing route projection (`routes.rs`).
2. Un-wrap `ReliableProvider` and restore crate-owned retry (`RunPolicy` retry is currently pinned to 1 attempt at seam `mod.rs:117-120` precisely because of the double-retry hazard).
3. Move the openhuman-backend billing/usage envelope parsing into the backend provider adapter (host-side `ChatModel` impl), not the generic wire client.
4. Delete: `compatible*.rs` (~6.6k L), `reliable.rs` (909 L, self-declared dead-in-waiting), `router.rs` (route via registry `ModelCatalog`), legacy `StreamChunk`/`stream_chat_*` streaming surface in `traits.rs` (superseded by `ProviderDelta`, itself superseded by crate streaming).
5. `compatible_tests.rs` (4,851 L / 207 tests — the richest suite in the domain): re-express the *behavioral* cases (SSE edge cases, tool-arg normalization, reasoning round-trip) against `providers/openai` fixtures upstream; keep host-side only the backend-envelope tests.
6. `Provider` trait stays temporarily as the host-side seam for factory/local/claude-code providers, now thin; evaluate retiring it entirely once all consumers are `ChatModel`.
7. **Exit:** one wire client (the crate's); `inference/provider/` reduced to factory + app-specific providers + ops/RPC.

### Phase 4 — Orchestration consolidation

1. Route the detached sub-agent control plane through `graph::orchestration` for real: replace `running_subagents.rs`'s bespoke registry/steer/abort/ownership machinery with `TaskStore` + `SteeringRegistry` (+ upstream anything missing: ownership enforcement, soft-cap sweep — evaluate as crate PRs). The seam's `orchestration.rs` stops being a partial re-export.
2. Replace `AgentOrchestrationSession` (`ops.rs`, 679 L) with crate `SubAgentSession` + `TaskStore`; consumers (`workflow_runs/engine.rs`, `agent_teams/runtime.rs`) move onto the crate API.
3. Unify status vocabularies (§2.5); host RPC keeps wire compat via mapping.
4. Reconcile `subagent_sessions/` (durable reusable sessions) with crate `JsonlTaskStore`/checkpoint reuse instead of a parallel JSON store.
5. Thin `spawn_parallel_graph.rs` further into the seam (it already runs on crate `map_reduce`); fix the two-spawn-path inconsistency (worktree diff results threaded in one path, hardcoded empty in the other — `ops.rs:559-562`).
6. Harden the `steering_forwarder.rs` (#4456) 50 ms poll-loop coupling upstream: give `SteeringHandle`/`RunQueue` bridging a first-class crate-side lifecycle instead of an abort-on-drop guard.
7. **Exit:** one sub-agent lifecycle (the crate's); `agent_orchestration/` reduced to product tools, event bridges, ledger, RPC.

### Phase 5 — Workflow/team generic slices (optional, evaluate after Phase 4)

1. Upstream the **validation** slices: workflow phase-DAG structural/cycle validation (`workflow_runs/{types,ops}`), team dependency validation + atomic CAS claim + quality-gate state machine (`agent_teams/ops.rs`) — as `graph/` extensions if they generalize cleanly; otherwise leave host-side. Durability (`session_db::run_ledger`) and `command_center/` stay host-side regardless.
2. `*_topology()` exports already funnel through seam `topology.rs::all_graph_topologies` — keep that pattern for anything upstreamed.

### Phase 6 — Cleanup & docs

1. Delete transitional shims (`ToolAdapter` test-only wrapper, `subagent_graph.rs` no-op skeleton once the graph path is the real one, `retrieve_tool_output` vs tokenjuice duplication).
2. Update `gitbooks/developing/architecture/agent-harness.md`, `orchestration.md`, module READMEs, and `about_app` if user-facing behavior shifted.
3. Final LOC/coverage ledger vs the Phase 0 baseline.

---

## 4. Test migration strategy

- **Port with the code (self-contained suites):** `schema_tests.rs` (15 fns), `generated_tests.rs` (24 fns), `error_classify` inline (35 tests), `model_context` inline, `parse.rs` inline (11), `worktree_tests.rs` (real-git, offline), filesystem/network tool inline suites, `policy.rs` inline. Re-express in tinyagents conventions: co-located `test.rs`, offline `MockModel`, `testkit` doubles, `e2e_*.rs` for integration, `live_*.rs` (key-gated) only where a real endpoint is essential.
- **Re-express behaviorally (harness-bound):** `compatible_tests.rs` (207 tests) → crate `providers/openai` fixture tests; `reliable_tests.rs` (26) → crate retry/resilience tests (most already exist — diff first, port the gaps).
- **Stay host-side:** `factory_tests.rs` (150), `ops_tests.rs` suites everywhere, `user_filter`/`orchestrator_tools` inline, all `agent_orchestration` harness-dependent suites (`engine_tests`, `runtime_tests`, `spawn_parallel_agents_tests`, `tools_e2e_tests`), all `/tests/*_e2e.rs` whole-app harnesses. These keep guarding the host wiring after cutover — they are the regression net for each deletion phase.
- **Coverage gates:** tinyagents expects ≥80%; host PR lanes gate ≥80% on changed lines. Cutover PRs that mostly *delete* code should still touch the seam with tests proving the crate-backed path preserves behavior (golden transcripts via `MockModel` scripts are the cheap way).

---

## 5. Bugs, gaps, and improvements found during the audit

Fix-in-place candidates (independent of the port; several become moot as phases delete their hosts):

**inference/**
- `presets.rs:200-205` — `recommend_tier` ignores its RAM input and always returns `MVP_MAX_TIER`; misleading dead logic (its own test asserts the non-scaling). Either implement scaling or rename/strip the parameter.
- `provider/traits.rs:734` — default `stream_chat_with_history` hardcodes `provider_name = "unknown"`, yielding the useless diagnostic `"unknown does not support streaming"`.
- `provider/traits.rs` — two parallel streaming abstractions coexist (`StreamChunk`/`stream_chat_*` legacy vs `ProviderDelta`); the legacy surface is near-dead and scheduled for deletion in Phase 3.
- `provider/reliable.rs` — live wrapper that no longer retries (pinned to one attempt); dead-code-in-waiting.
- `provider/factory.rs:404` — `NO_MODEL_CONFIGURED_ANCHOR` correctness depends on a string literal matched by a separate classifier in `provider/ops.rs:19`; fragile cross-file coupling.

**tools/**
- `tools/ops.rs` — `all_tools_with_runtime` takes 12 positional args (`#[allow(clippy::too_many_arguments)]`; the thin `all_tools` shim forwards 9); a builder is overdue. The `Vec<Box<dyn Tool>>` container doesn't dedupe (a test exists *because* of this); the crate's `ToolRegistry` fixes it structurally.
- `tools/traits.rs:15` — `ToolScope::AgentOnly` is a dead variant; `ToolCategory::Workflow` is pinned to wire `"skill"` (documented tech-debt to resolve before porting the type).
- `tools/traits.rs:367-372` — `is_concurrency_safe` is advisory-only (harness runs tools serially); tinyagents 1.6 has concurrent independent tool calls — adopting it makes the flag real.
- `orchestrator_tools.rs:38-41,87-89` — dead `SpawnWorkerThreadTool` registration (pending #1624); mis-attached doc-comment at lines 592-603 describes a different test than the one it precedes.
- `user_filter.rs:168-188` — `skill_manage` and `workflow_manage` families carry identical `rust_names` (deliberate alias, duplication footgun); `web_search` → `"web_search_tool"` name mapping is drift-prone against the `search` domain.
- `impl/network/polymarket*` (~2.5k L incl. tests) — a large app integration living under the "cross-cutting families only" `impl/` rule; should move to a domain per the module's own README.
- Seam `tools.rs` — `tool_policy_from_openhuman_tool` silently drops `category`, `scope`, and all `*_with_args` per-call gating (§2.2).

**agent_orchestration/**
- `ops.rs:138-168` — `message_agent` records metadata only and never injects into the running loop; a real functional hole that `agent_teams/runtime.rs` and `command_center` inherit.
- `subagent_control.rs` — the `SteerError::NotOwned` arm is unreachable on the trusted RPC `steer_control` path (which performs no parent-ownership check), but the variant is **not dead**: the agent-tool `running_subagents::steer` path enforces ownership and returns it (guarded by `steer_rejects_cross_parent_and_unknown`). Real RPC-path enforcement is deferred to Phase 4 (`SteeringRegistry` ownership consolidation).
- `ops.rs:559-562` — in-memory session path hardcodes `worktree_path: None, changed_files: []` on completion even for worktree-isolated workers (inconsistent with `spawn_parallel_graph`).
- `delegation.rs:120-127` — human-approval interrupt permanently disabled; the durable interrupt path in the seam is wired but unreachable until a review surface exists.
- Two parallel status enums (`AgentStatus` vs `OrchestrationTaskStatus`) — latent drift hazard (§2.5).
- Process-wide `OnceLock<Mutex<…>>` singletons in `background_completions`/`background_delivery`/`running_subagents` serialized through `TEST_ENV_LOCK` in tests — the crate favors injected stores; Phase 4 removes most of them.

**seam (`src/openhuman/tinyagents/`)**
- `mod.rs:117-120` — retry pinned to 1 attempt (Workstream 11 debt; resolved in Phase 3).
- `orchestration.rs:23-33` — bridge is re-export-only; live control path still on `RunQueue` (resolved in Phase 4).
- `retriever.rs:14-27` — crate `Retriever`/`InMemoryVectorStore` built but unused on the live path (dead-until-swap; out of scope here, note for the memory migration).
- `steering_forwarder.rs` (#4456) — abort-on-drop guard papering over a fragile poll-loop coupling; harden upstream (Phase 4.6).
- `middleware.rs` — prompt-cache drift detection duplicated between seam and crate `PromptCacheGuardMiddleware`, with differing fingerprints until the 1.6.0 bump (Phase 0).

---

## 6. Risks & mitigations

| Risk | Mitigation |
| --- | --- |
| Behavioral drift cutting `compatible*.rs` over to crate providers (SSE edge cases, tool-arg normalization, backend envelope) | Provider-by-provider cutover behind the route projection; port the 207-test behavioral suite upstream *before* deleting; keep backend-envelope parsing host-side |
| Double-retry / retry semantics during Phase 3 transition | Keep the 1-attempt pin until `ReliableProvider` is un-wrapped in the same PR that enables crate retry |
| Lossy tool-metadata mapping ossifying (permissions, MCP blocks) | §2 design decisions land as a tinyagents PR **before** any tool family moves |
| tinyagents is public crates.io GPL code | Audit each upstream PR for product policy/backend phrase coupling; backend-specific classifiers stay host-side as extension tables |
| Version skew between submodule pin, crates.io version req, and upstream tags | Every bump is a standalone `chore(vendor)` commit pinning tag + version together; never float the submodule |
| The two-Cargo-world topology (root crate vs Tauri shell, #3877) means two lockfiles to bump | Bump both manifests in the same `chore(vendor)` commit; CI full lane validates both |
| Big-bang deletion breaking the fast PR lane's changed-file coverage gate | Phase PRs pair each deletion with seam tests exercising the crate-backed path |

---

## 7. Expected outcome (rough accounting)

- **Deleted from host:** `compatible*.rs` + tests (~11.5k), `reliable.rs` + tests (~2k), `router.rs` (~0.8k), orchestration session + detached registry + parallel-graph thinning (~4–5k), tool families moved (~10k incl. tests), misc (streaming legacy, shims) — **~30k LOC** leaves the host across Phases 1–4.
- **Added to tinyagents:** builtin tools (feature-gated), schema cleaning, error classification, reasoning channel, worktree isolation, context-window table, orchestration hardening — **~12–15k LOC** upstream, most of it accompanied by ported tests.
- **Unchanged:** RPC wire surface, local AI runtime, voice, provider factory grammar, product tool catalog, run ledger, UI.
