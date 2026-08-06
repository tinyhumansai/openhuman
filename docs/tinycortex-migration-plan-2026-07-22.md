# TinyCortex Migration — Current-State Audit & Consolidated Plan (2026-07-22)

**Status:** **DONE** on `feat/tinycortex-migration-2026-07-22`.
[CI Full run 29925645209](https://github.com/senamakel/openhuman/actions/runs/29925645209)
is green, including the final gate and every Linux, macOS, and Windows desktop
shard
(companion to `tinyagents-migration-plan-2026-07-22.md`;
supersedes the *status/phasing* in `tinycortex-memory-migration-plan.md`,
`tinycortex-migration-spec.md`, `tinycortex-api-gap-audit.md`, and
`tinycortex-parity-checklist.md` — those docs' ownership splits, gap lists, and
parity dimensions remain the reference detail. `tinycortex-drift-ledger.md`
stays the row-level ledger but its anchors are stale — see §2).
**Scope:** finish moving the remaining generic memory-engine code from
`src/openhuman/` into the vendored `tinycortex` crate (`vendor/tinycortex`),
delete the in-tree duplicates and staging code, migrate/retire the affected
tests, and clean up every dangling doc/code reference.
**Method:** fresh four-way audit — (1) the crate surface, (2) the core memory
domains, (3) the periphery + the `src/openhuman/memory/tinycortex/` seam, (4) the
docs/tests/git history — against the working tree at `main` (`5b8a9f269`,
2026-07-22).

---

## 1. Executive summary

**The engine migration is essentially done and the docs don't know it.** All
five `docs/tinycortex-*` docs self-describe a "W1–W2 landed; W3 partial" state,
but git shows the full W-track landed: W1 seam (#4526), W2 type re-export
(#4529), the W3 chunk-store cutover chain (#4532–#4559), W4 queue flip
(#4781), W5 vector+scoring (#4790), the W7 long-tail shims (#4785–#4789),
**engine-migration completion (#4794)**, **engine test port into the crate
(#4820)**, and post-completion feature work (persona/coding-session ingest
#4863) on top. The drift ledger's D1–D3 rows are all CLOSED.

What remains is not "migrate the engine" but five bounded packages (§5):

1. **Housekeeping** — the pin/tag story is broken (crate has *no git tags*
   despite the Cargo.toml comment demanding tag-lockstep), the host CI vendor
   suite omits the `persona` feature the host enables, and there are ~8
   broken/stale references including a seam module doc naming files that were
   never created.
2. **Delete `memory_store/unified/`** — the explicitly "staging for removal"
   tier (~5k LOC + ~3.7k test LOC), gated by the G1 `sqlite_conn()` escape
   hatch.
3. **W-SYNC completion** — `memory_sync/` (17.7k LOC, the largest remaining
   host mass) still runs the host provider pipelines while the crate now ships
   a full `memory::sync` Composio engine; the D4 ledger rows gate the flip.
4. **Embedding-provider dedupe** — host `embeddings/` adapters and
   `memory_tree/score/embed/` providers now duplicate what
   `tinyagents::harness::embeddings` ships since tinyagents #58 (this package
   lands in **tinyagents**, not tinycortex — the crates split the job).
5. **Shim retirement + funnel decision** — the compatibility shims left behind
   by W3–W7, and a policy call on the fact that domains import
   `tinycortex::memory::*` directly while the seam's re-export facade has
   **zero** consumers.

---

## 2. Ground truth: pins, versions, and discrepancies (fix first)

| Fact | Value | Where |
| --- | --- | --- |
| Host requirement | `tinycortex = { version = "0.1", features = ["git-diff", "persona", "sync"] }` | root `Cargo.toml:116` |
| Path override | `[patch.crates-io] tinycortex = { path = "vendor/tinycortex" }` | `Cargo.toml:682` |
| Vendored crate | `0.1.1`, edition 2021, **MIT** (not GPL — unlike tinyagents), repo `tinyhumansai/tinycortex` | `vendor/tinycortex/Cargo.toml` |
| Submodule HEAD | `daaaf6ba` (`heads/main`, dependabot-era, ~50+ commits past every SHA the docs cite) | `git submodule status` |
| Tags | **none exist** — `git describe --tags` fails; yet host `Cargo.toml:115` says "Keep the version pin in lockstep with the submodule tag" | submodule |
| Docs' claimed pin | gitlink `a8e10f7` (exists in history, long since passed) | drift ledger / spec anchors |
| Nested dep | tinycortex requires `tinyagents = { version = "2", path = "vendor/tinyagents" }`, **its own nested submodule pinned at v2.0.0** | `vendor/tinycortex/Cargo.toml:69` |
| Host tinyagents req | `2.1` | root `Cargo.toml:107` |
| Fork patch | `[patch."https://github.com/senamakel/tinyagents"]` exists *for tinycortex's sake* ("temporarily pins the embedding API port while tinyagents #58 is pending" — #58 is now merged at the tinyagents submodule HEAD) | `Cargo.toml:685-691` |
| Host CI vendor suite | `cargo test --manifest-path vendor/tinycortex/Cargo.toml --features git-diff,sync` — **omits `persona`**, which the host enables | `.github/workflows/ci-lite.yml:859` |

Version skew summary: three different tinyagents expectations (host `2.1`,
tinycortex `2`, nested submodule `2.0.0`) resolve only via path patches. The
tinyagents plan's WP-0 (bump `vendor/tinyagents` to v2.1.0) should be executed
together with bumping tinycortex's *nested* `vendor/tinyagents` so both crates
share one trait identity at one version, after which the `senamakel` patch
block and its "temporarily pins" comment can likely be retired.

---

## 3. What the crate already ships

`tinycortex 0.1.1` exposes a single `memory` module — the engine "ported from
OpenHuman" — with features `tokio`, `git-diff` (git2-backed `memory::diff`),
`providers-http` (reqwest embedding/LLM providers, host does **not** enable),
`sync` (live Composio + workspace-scan engine), `persona` (doc-06 persona
distillation). Modules: `store` (content/vectors/kv/entity_index/safety+PII),
`chunks`, `sources` (+readers), `score` (embed/extract/signals), `tree`
(+runtime, bucket/document seal, flavoured trees), `queue` (SQLite job queue +
tokio runtime), `retrieval` (hybrid search, MMR, rerank), `entities`, `graph`,
`goals`, `tool_memory`, `conversations`, `archivist`, `ingest`
(canonicalize→chunk→score→tree), `diff`, `sync` (composio
clickup/github/linear/notion/slack/gmail), `providers`, `persona`
(readers for claude_code/codex/git history), `fsutil`.

Host-facing extension points (~20 public traits) with the seam's current
implementations in parentheses: `Memory`, `MemoryStore`, `EmbeddingBackend` +
`Embedder` (`SeamEmbedder`), `ChatProvider` (`SeamChatProvider`), `Summariser`
(`HostSummariser`), `SealObserver` + seal-time `Embedder` bridge (`seal.rs`),
`TreeJobSink` (`HostTreeJobSink`), `QueueDelegates` (`HostQueueDelegates`),
`SourceReader`, `SelfIdentity`, `EntityOccurrenceIndex`, `SnapshotItemSource`,
`GoalsGenerator`, `ConversationEventBus`/`ChannelEventHandler`,
`PersonaStateStore`, and the sync seams `SyncStateStore` / `SyncEventSink` /
`SkillDocSink` / `LocalDocumentSink` / `ExternalSourceReader` / `SyncPipeline`
/ `ActionExecutor` / `IncrementalSource` (`sync.rs`, `HostSyncAdapter`).

The crate consumes `tinyagents::harness::embeddings::{EmbeddingModel,
format_embedding_signature}` in `store/vectors/embedding.rs` — that is the
whole tinyagents dependency (the W-EMB decision made real by tinyagents #58).

Crate CI: fmt, clippy `-D warnings`, build+test `--all-features`, doc build
with warnings denied, plus a per-feature matrix (`core`, `tokio`, `git-diff`,
`sync`, all). Offline by convention; Composio/live tests are wiremock-mocked or
env-key-gated.

---

## 4. Audit: what remains host-side

### 4.1 Per-domain state (crate-backed file counts via `use tinycortex::`)

| Domain | Files / LOC | Crate-backed | State |
| --- | --- | --- | --- |
| `memory/` | 69 / 18.0k | 3 | Orchestration + policy + RPC layer (by design "no SQLite here"). Predominantly host: `read_rpc/`, `schema*/`, `rpc_models`, agent tools, sync orchestration, tree *policy*. Stays, minus shim cleanup. |
| `memory_store/` | 54 / 17.7k | 15 | Sub-stores flipped to crate (chunks, content, vectors, kv, entities, trees, safety/PII per W3/W7 markers). **`unified/` (~5k + ~3.7k tests) is README-marked "Staging for removal"** — documents/query/segments/events/profile await per-kind flips. `factories.rs` (config/provider selection) stays host. |
| `memory_tree/` | 47 / 10.3k | 16 | Largest crate-backed surface; engine parts delegate. Host keeps RPC (`tree/rpc.rs` 1,387, `retrieval/rpc.rs`), CLI, bus subscriber, `health/` doctor (G5, host-retained by decision), and `score/embed/` **providers** (~1.9k — see WP-3). |
| `memory_queue/` | 7 / 1.4k | 2 | W4 flipped: `worker.rs` delegates claim→dispatch→settle to `tinycortex::memory::queue::run_once` via `HostQueueDelegates`; per-kind handlers deleted. Host keeps worker pool, scheduler tick, `scheduler_gate` throttle, SQLite glue. Done, minus the `memory::jobs` re-export shim. |
| `memory_search/` | 8 / 0.8k | 2 | W5 flipped: `scoring.rs`/`vector/mmr.rs` are near-empty shims over crate `WeightProfile`/`retrieval::mmr`. Host keeps the three agent tools. Done. |
| `memory_tools/` | 9 / 1.4k | 3 | W7 flipped: types/store/prompt are shims over `tool_memory::{types,store,render}`. Host keeps `capture.rs` (`PostTurnHook`) + put/list tools. Done. |
| `memory_archivist/` | 1 / 44 | 1 | Pure W7 shim over `memory::archivist`. Done. |
| `memory_conversations/` | 2 / 0.9k | 2 | W7 shim over `memory::conversations`; host keeps `bus.rs` event-bus glue (+ a `tinychannels` legacy-message conversion). Done. |
| `memory_diff/` | 7 / 2.0k | 11 uses | W7 flipped: engine crate-owned; host keeps RPC/tools + `DomainEvent::MemoryDiff*` publishing. Done. |
| `memory_goals/` | 6 / 0.9k | 4 uses | W7 flipped: types/store crate-owned (byte-identical `MEMORY_GOALS.md`); host keeps agent wiring. Done. |
| `memory_sources/` | 16 / 5.0k | 12 uses | Product registry (config-persisted connectors) with crate readers underneath. Host by design. |
| `memory_sync/` | **73 / 17.7k** | 5 uses | **The largest remaining mass.** Host Composio provider pipelines (gmail/slack/github/notion/linear/clickup) still run host-side while the crate's `memory::sync` ships the same provider engine. W-SYNC seam (`sync.rs`, 679) is landed; the host flip (W-SYNC.3) has not happened. See WP-2. |
| `embeddings/` | 13 / 3.1k | **0** | Bridges **tinyagents** (9 files), not tinycortex. Post-#58, the host per-provider adapters duplicate crate `harness::embeddings` providers. See WP-4. |
| `agent_memory/`, `subconscious/` (memory parts), `learning/` (memory parts) | — | 0 | Host product consumers of the memory stack; no crate duplication. Stay. |

Consumer counts (host memory tiers are deeply embedded and *stay* host, so
these are context, not deletion blockers): `memory::` 166 files,
`memory_store::` 132, `memory_tree::` 67, `memory_queue::` 18,
`memory_tools::` 11, `memory_search::` 3.

### 4.2 The seam — `src/openhuman/memory/tinycortex/` (11 files, ~3.2k LOC)

`config.rs`, `embeddings.rs`, `chat.rs`, `summariser.rs`, `ingest.rs`,
`seal.rs`, `sync.rs` (679), `queue_driver.rs` (1,007), `persona.rs` (446),
`parity.rs` (`#[cfg(test)]` format pins), `mod.rs`. Healthy and intentionally
host-owned. Two findings:

- **The seam is not the funnel.** `mod.rs` re-exports crate types
  (`MemoryEntry`, `MemoryTaint`, `RecallOpts`, …) as the intended
  type-unification funnel, but **zero** files import `crate::…::tinycortex::`
  re-exports — every memory domain imports `tinycortex::memory::*` directly.
  Either convention works; pick one and record it (WP-5). Direct crate imports
  are simpler and match how `tinyagents` is consumed; if so, shrink the facade
  instead of promoting it.
- **Stale internal doc:** `mod.rs:28-30` names sibling adapters `sinks.rs` and
  `bus.rs` that were never created (their roles landed as
  `ingest.rs`/`seal.rs`/`sync.rs`). `queue_driver.rs:21-26` still says "This
  brick is additive: nothing is flipped yet" — the flip landed in #4781.

Coupling is bidirectional by design (seam reaches back into
`memory_store`/`memory_tree`/`memory_sync` to do host work inside crate
callbacks); not a defect, but `queue_driver.rs`'s 11 host imports will thin as
WP-1/WP-3 shims retire.

### 4.3 Broken/dangling references (resolved in WP-0)

| Resolved issue | Resolution |
| --- | --- |
| Missing TinyAgents companion-spec link | Replaced with the existing July 22 migration plan. |
| Missing standalone deletion-ledger link | Replaced with §2 of the TinyCortex migration spec. |
| Wrong agent-harness documentation path | Pointed at the architecture subdirectory. |
| Never-landed golden-workspace generator | Removed after the post-cutover parity decision descoped synthetic pre-cutover fixtures. |
| Wrong raw-coverage test path | Corrected to the `tests/raw_coverage/` location. |
| Stale line anchors `Cargo.toml:82`, `src/openhuman/mod.rs:130` | `tinycortex-migration-spec.md` §0.4 (actual: `Cargo.toml:116`, `mod.rs:140`) |
| Never-created seam sibling names | Replaced with the adapters that actually landed. |
| Developer-specific absolute path in crate docs | Replaced with repository-relative paths. |

Also stale-in-place: all five docs' status headers (pre-#4794), the drift
ledger's pin anchors, and the crate migration doc's claim that "TinyCortex
will not own memory sync" (superseded by the plan's §8 sync-inclusive
amendment and the crate's shipped `sync` module — the *scheduler/credentials/
RPC* stay host, the *engine* is crate; restate the boundary in one place).

### 4.4 Test inventory

- **In-crate (done):** engine coverage was ported by #4820; the crate runs its
  own suite offline (wiremock Composio mock; live tests env-gated). W8's CI
  wiring exists: `ci-lite.yml:859` runs the vendor suite on submodule-pointer
  changes — **but with `--features git-diff,sync` only; `persona` is untested
  in host CI while enabled in the host build.**
- **Dies with WP-1:** `memory_store/unified/*_tests.rs` (~3.7k LOC:
  `documents_tests` 1,275, `query_tests` 1,000, `profile_tests` 763,
  `segments_tests` 393, `events_tests` 287) go with the unified tier; port any
  still-unique assertions to the per-kind crate backends first.
- **Retargets with WP-2:** the memory_sync suites (389 inline tests across 35
  files + provider `*_tests.rs`: github 655, gmail 301+354, slack 180, linear
  172, clickup 155, notion 119) — post-flip, provider *engine* behavior is
  crate-tested; host keeps orchestration/bus/config tests. Integration:
  `memory_sync_pipeline_e2e.rs` (570), `memory_golden_parity_e2e.rs` (275),
  `raw_coverage/memory_sync_*` retarget to the crate-backed path.
- **Golden parity harness:** comparators 1 & 5 green; 2/3/4 TODO;
  `frontmatter_parity` Layer-1 asserter pending; fixture generator script
  missing. Decide finish-vs-descope in WP-0 (post-cutover, parity pins matter
  mainly for `MemoryTaint`/on-disk stability — keep those, consider dropping
  the rest).
- **Stays host:** `json_rpc_e2e.rs` memory sections, `memory_sources_e2e`
  (837), `memory_tree_summarizer_e2e` (583), `memory_roundtrip_e2e`,
  `memory_graph_sync_e2e`, `memory_fast_retrieve_e2e`, `autocomplete_memory_e2e`,
  `memory_artifacts_e2e`, the 15 `raw_coverage/memory_*` modules, and all
  subconscious/learning suites.

---

## 5. The plan — five work packages

Same conventions as the tinyagents companion plan: WP-0 first (hygiene,
unblocks honest review), then largely independent packages; drift-ledger rows
before deletions; failing-before/passing-after regression tests; small
validated commits on feature branches per submodule.

### WP-0 — Version/tag + CI + docs housekeeping (no engine behavior change)

1. **Establish the tag discipline the Cargo comment promises:** tag
   `vendor/tinycortex` (e.g. `v0.1.1` at the current release point, or cut
   `v0.2.0` at HEAD), and record the bump workflow the plan prescribed
   (`chore(vendor): bump tinycortex …` — zero such commits exist in host
   history). Alternatively amend the comment to describe reality
   (main-tracking submodule). Either is fine; the current state (comment
   demands tags, none exist) is the worst option.
2. **Align the nested tinyagents:** when the tinyagents plan's WP-0 bumps
   `vendor/tinyagents` to v2.1.0, bump `vendor/tinycortex/vendor/tinyagents`
   in lockstep and update tinycortex's `tinyagents = "2"` if needed; then
   evaluate retiring the `[patch."…senamakel/tinyagents"]` block and its
   obsolete "while tinyagents #58 is pending" comment (#58 is merged).
3. **Close the persona CI hole:** add `persona` to the vendor-suite features
   in `.github/workflows/ci-lite.yml:859` (and confirm the crate's own
   feature-matrix covers `persona`, which today it does not list either).
4. **Docs refresh:** update the five docs' status headers to post-#4794/#4820
   reality; update drift-ledger anchors (pin `daaaf6ba`+tag, note #4794/#4820/
   #4863); document the `persona` feature + seam (currently invisible in all
   five docs); restate the sync ownership boundary once (engine=crate,
   scheduler/credentials/RPC/bus=host) and fix the crate-side migration doc's
   contradictory "will not own sync" line + its absolute local path.
5. **Fix the §4.3 broken references**, including the two code-adjacent ones:
   correct the seam module inventory and the queue-driver's stale pre-flip
   description.
6. **Golden-parity decision:** finish comparators 2/3/4 + `frontmatter_parity`
   + the fixture script, **or** descope to the security-relevant pins
   (`MemoryTaint` byte-identity, chunk/vector on-disk format) with the
   decision recorded in the parity checklist.

**Exit:** pins/tags/CI/docs agree with the tree; `grep` for the phantom paths
comes back clean.

### WP-1 — Retire `memory_store/unified/` (the "staging for removal" tier)

1. Inventory the five remaining unified facets (documents, query, segments,
   events, profile — ~5k LOC) against their per-kind crate backends; flip
   callers facet-by-facet (the W3 sub-store pattern, already proven for
   chunks/content/vectors/kv/entities/trees/safety).
2. The blocker is **G1** (`sqlite_conn()` escape hatch, ~312 refs at audit
   time): re-audit the current count, then either land the crate-side access
   the gap audit sketched or explicitly re-scope G1 to the host-retained
   namespace-document tables and record it.
3. Decide the fate of the 10-table host-retained `UnifiedMemory`
   namespace-document tier: keep host (spec's standing decision) but move it
   out of `unified/` into a clearly-named home so "staging for removal" can
   actually be removed.
4. Tests: port unique assertions from the ~3.7k LOC of `unified/*_tests.rs`
   to per-kind backends (crate-side where engine behavior, host-side where
   glue), then delete with the code.

**Exit:** `memory_store/unified/` gone or reduced to the renamed host tier;
`memory_store/` ≈ 9–10k LOC of glue + host tier; `memory_roundtrip_e2e` +
`memory_golden_parity_e2e` + full mock-suite green.

### WP-2 — W-SYNC.3: flip `memory_sync/` onto the crate sync engine

The seam (`sync.rs`: `ExternalSourceReader`, `SkillDocSink`,
`LocalDocumentSink`, `SyncStateStore`, `SyncEventSink`) is landed; the crate
ships the Composio engine incl. per-provider modules. Remaining:

1. Close the **D4.1–D4.4** drift rows (ledger requires this before the flip).
2. Flip provider-by-provider (gmail first — the crate already carries the
   Gmail 413-page fix #73), keeping host-side: scheduler/periodic trigger,
   credentials, `config.toml` source registry (`memory_sources/`), event-bus
   publishing (`ChannelMessageReceived` et al.), RPC/status surface, and
   redaction/`source_scope` gating.
3. Dedupe post-processing: host `providers/*/post_process*` vs crate provider
   modules — upstream generic normalization, keep host product policy.
4. Tests per §4.4: engine behavior moves to crate suites (wiremock Composio),
   host keeps orchestration/bus/e2e; `memory_sync_pipeline_e2e` retargets.

**Exit:** `memory_sync/` shrinks from 17.7k toward ~6–8k LOC of
orchestration/product glue; D4 rows CLOSED; no duplicated provider parsing.

### WP-3 — Embedding-provider dedupe (lands in *tinyagents*, coordinated here)

Two host clusters now duplicate `tinyagents::harness::embeddings` (post-#58:
openai/cohere/voyage/ollama/cloud + rate-limit/retry-after):

1. `src/openhuman/inference/embeddings/` adapters (`openai/voyage/cohere/ollama/
   cloud_adapter.rs`, `provider_trait.rs`) → construct crate
   `EmbeddingModel`s directly; host keeps `rpc.rs` (1,479), `schemas.rs`,
   `catalog.rs`, `factory.rs` (config/BYOK selection, #4056 dimension
   gating).
2. `memory_tree/score/embed/{openai_compat,ollama,cloud,factory}.rs` (~1.9k)
   → same crate models via the seam's `SeamEmbedder`/`EmbedderBridge`.

Because tinycortex consumes the same `EmbeddingModel` trait, this single
change serves both crates — it is the concrete payoff of the W-EMB decision.
Sequence after WP-0's tinyagents version alignment.

**Exit:** one embedding-provider implementation per provider across the
workspace; `embeddings_rpc_e2e` (822) + `ollama_embeddings_fallback_e2e` (234)
green against the crate-backed path.

### WP-4 — Shim retirement + funnel decision

1. **Funnel policy:** adopt direct `tinycortex::memory::*` imports as the
   convention (matches reality and the tinyagents precedent), shrink the seam
   `mod.rs` facade to the types host code genuinely re-brands
   (`MemoryTaint` stays pinned by parity tests) — or the reverse; either way,
   record it in the spec and stop carrying both.
2. Retire compatibility shims whose consumers can move: `memory_tree/tools.rs`
   (points at `memory`), `memory_store/trees` legacy paths, the
   `memory_queue`→`memory::jobs` re-export, `memory_archivist` (44-line shim —
   move its 2 call sites and delete the domain), and the W5/W7 near-empty
   shims once import counts hit zero (`memory_search/scoring.rs`,
   `vector/mmr.rs`, `memory_tools/{types,store}.rs`).
3. Thin `queue_driver.rs` (1,007) as WP-1 removes its `memory_store`
   reach-backs.

**Exit:** shim count measurably down (each deletion = one commit with its
import-migration); seam ≤ ~2.5k LOC; no module whose entire body is a
re-export except deliberate facades recorded in the spec.

### WP-5 — Exit gate

- Full suite: `scripts/test-rust-with-mock.sh` (incl. the 15
  `raw_coverage/memory_*` modules), vendor suite
  `cargo test --manifest-path vendor/tinycortex/Cargo.toml --features
  git-diff,sync,persona`, slim disabled build (`--no-default-features
  --features tokenjuice-treesitter` — memory domains are ungated but the
  standing repo rule applies), `pnpm rust:check`.
- Drift ledger: D4 CLOSED; new rows for every WP-1/WP-2 deletion; the spec's
  §2 deletion-ledger skeleton filled in with actuals.
- Docs: the five tinycortex docs + `src/openhuman/memory*/README.md` files +
  `gitbooks/developing/architecture.md` reflect the post-flip reality; this
  plan stamped done.

### Execution record (2026-07-22)

| Package | Result |
| --- | --- |
| WP-0 | TinyAgents unified at 2.1 across OpenHuman, TinyCortex, and TinyFlows; obsolete fork patch removed; TinyCortex CI covers `persona`; docs and phantom references repaired. TinyCortex intentionally tracks reviewed upstream commits rather than nonexistent tags. |
| WP-1 | G1 count is zero. The ten-table namespace/document product tier is host-owned, so the complete `unified/` directory was renamed to `namespace_store/` instead of deleted as engine duplication. Its 153 targeted tests pass. |
| WP-2 | The live default path was already TinyCortex-backed. D4 is CLOSED; dead Gmail duplicate removed; remaining provider task/profile projections explicitly classified as product policy. Provider tests pass (301). |
| WP-3 | Concrete provider transports deduplicated into TinyAgents. The memory tree now uses a thin `ProviderEmbedder`; Ollama's 8k context/batch and missing-model guidance moved upstream before the host client was deleted. TinyAgents embedding tests pass (38); host library check passes. |
| WP-4 | Archivist, search scoring/MMR, tool-memory type/store, tree-tool, jobs-alias, and unused seam type facades retired. Direct crate imports are canonical. Seam production code is 2,229 LOC (below the 2.5k exit target). |
| WP-5 | Local focused validation is recorded above; the slim `--no-default-features --features tokenjuice-treesitter` build also passes. [CI Full run 29925645209](https://github.com/senamakel/openhuman/actions/runs/29925645209) is green: core quality and full tests, TinyCortex, Tauri, frontend, mock-backend Rust E2E, Playwright, three desktop builds, every launched desktop shard, and the final gate. Two first-attempt Linux jobs ended without uploaded logs; rerunning only failed jobs passed the Linux build, Rust integration suite, and all eight Linux shards. |

---

## 6. Risks and standing gotchas

- **`MemoryTaint` is security-critical.** The type is proven byte-identical
  host↔crate and fails closed to `ExternalSync`; the parity pins in
  `tinycortex/parity.rs` are the regression guard — whatever WP-0 decides
  about the golden harness, **keep these**.
- **Redaction and `source_scope` gating stay host.** WP-2 must not let crate
  sync code become a path around host redaction or the source allowlist —
  review the `SyncEventSink`/`LocalDocumentSink` implementations for every
  provider flipped.
- **On-disk compat:** chunk/vector formats, `MEMORY_GOALS.md` byte-identity,
  and the P5 fresh-DB divergence (3 legacy inline-embedding columns in
  `mem_tree_chunks`) — migrations of user data must be forward-only and
  tested against a pre-migration workspace fixture.
- **MIT vs GPL:** tinycortex is MIT (tinyagents is GPL-3.0) — moving host code
  *down* into tinycortex relicenses it; fine for our own code, but keep the
  "no product policy / no keys / nothing openhuman-branded as API" rule, and
  don't move anything derived from GPL-only sources into the MIT crate.
- **Two Cargo worlds + nested submodule:** vendor bumps now touch up to three
  lockfiles (root, `app/src-tauri`, and tinycortex's own) and two submodule
  pointers (`vendor/tinycortex`, `vendor/tinycortex/vendor/tinyagents`). CI
  clones with `submodules: recursive` for the vendor suite — verify after any
  bump.
- **Event-bus surface:** `memory_sync` publishes 7 distinct `DomainEvent`
  variants; the WP-2 flip must keep publishing them from host callbacks (the
  crate has no event bus by design).

---

## 7. Quick reference — disposition of every audited module

| Module | Verdict |
| --- | --- |
| `memory_store/unified/` | DELETE after per-kind flips; host tier re-homed (WP-1) |
| `memory_store/` sub-stores, `factories.rs` | DONE (crate-backed) / STAYS (host glue) |
| `memory_sync/` provider engines | FLIP to crate `memory::sync` (WP-2) |
| `memory_sync/` scheduler, bus, RPC, credentials | STAYS |
| `embeddings/` per-provider adapters | REPLACE with `tinyagents::harness::embeddings` models (WP-3) |
| `embeddings/` rpc/schemas/catalog/factory | STAYS |
| `memory_tree/score/embed/` providers | REPLACE via seam + crate models (WP-3) |
| `memory_tree/` engine files | DONE (crate-backed); RPC/CLI/bus/health STAY |
| `memory_queue/`, `memory_search/`, `memory_tools/`, `memory_diff/`, `memory_goals/`, `memory_conversations/` | DONE — retire residual shims when import counts reach zero (WP-4) |
| `memory_archivist/` | DELETE the 44-line shim after moving 2 call sites (WP-4) |
| `memory/` (orchestration/RPC/tools/tree-policy) | STAYS |
| `memory_sources/` | STAYS (product registry over crate readers) |
| `agent_memory/`, `subconscious/`, `learning/` | STAYS (consumers, no duplication) |
| Seam `src/openhuman/memory/tinycortex/` | STAYS, shrinks; fix stale module docs (WP-0), thin `queue_driver.rs` (WP-4) |
| Golden-parity harness | FINISH or DESCOPE by decision (WP-0); `MemoryTaint`/format pins KEEP |
