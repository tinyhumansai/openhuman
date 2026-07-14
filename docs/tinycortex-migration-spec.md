# TinyCortex Memory Migration — Spec (Phase 0.5 / 0.6)

**Status:** Phase 0 baseline **+ execution underway** — #59 (native-dep alignment) merged and the dep
is active (§0.4); W1 seam + W2 type re-export + W3-chunks partial landed; **drift closure COMPLETE**
(**D3** via #59, **D2** via tinycortex#63, **D1** via tinycortex#64; gitlink `a8e10f7`) — so **W4
(queue) and W7 (conversations) are unblocked**. Anchors the migration to exact reviewed SHAs and ties
together the drift, gap, and parity ledgers. Modeled on `docs/tinyagents-migration-spec.md` + its
deletion ledger.

**Companion plan:** [`tinycortex-memory-migration-plan.md`](tinycortex-memory-migration-plan.md)
**Ledgers:** [`tinycortex-drift-ledger.md`](tinycortex-drift-ledger.md) ·
[`tinycortex-api-gap-audit.md`](tinycortex-api-gap-audit.md) ·
[`tinycortex-parity-checklist.md`](tinycortex-parity-checklist.md)

## Version anchors

| Repo | SHA | Note |
| --- | --- | --- |
| `tinyhumansai/openhuman` | `7850cf363559bcbb7ba688cbc4fccdb6bd9ce754` | host audit base (`main`, 2026-07-04) |
| `tinyhumansai/tinycortex` | `d1a8c7be2babc8fff7a72ed93861f459f3d6fa58` | crate audit base (v0.1.1) |
| `tinyhumansai/tinycortex` | `33dda943053e61ef585fc39647cf1854344b6323` | audit base **+ #59** (native-dep alignment, §0.4) merged |
| `tinyhumansai/tinycortex` | `a8e10f7dd8ebdb9b0905e1380fefcc6bf5a65207` | **current pinned gitlink** — **+ #63/#64** (D2/D1 drift closure) merged; all drift rows CLOSED |

Port line (derived by content, §0.1): **after 2026-06-25, before 2026-06-28** for engine features.

---

## 0.5 — Type-unification decision

**Decision: host re-exports the crate types** (`pub use tinycortex::memory::{…}` from
`memory/traits.rs`), the preferred option in the plan. One source of truth; 30+ consumer sites keep
their import paths through the re-export. Rationale confirmed by 0.3 (wire-compatible on disk) and the
`MemoryTaint` proof below. Fallback (host types + `From` conversions) is **not** needed — no
serde/API divergence was found.

### MemoryTaint — security-critical, proven identical (required before re-export)

`MemoryTaint` drives external-effect-tool gating (a tainted subconscious turn must refuse
`external_effect` tools). Its serde form, db strings, and fail-closed default were compared
byte-for-byte:

| Property | Host (`memory/traits.rs:25`) | Crate (`types.rs:26`) | Match |
| --- | --- | --- | --- |
| Variants | `Internal` (`#[default]`), `ExternalSync` | `Internal` (`#[default]`), `ExternalSync` | ✅ |
| serde | `snake_case` | `snake_case` | ✅ |
| `as_db_str` | `internal` / `external_sync` | `internal` / `external_sync` | ✅ |
| `from_db_str` unknown | → `ExternalSync` (fail-closed) | → `ExternalSync` (fail-closed) | ✅ |

The **more restrictive** taint is the default and the unknown-decode target on both sides — the
fail-closed-to-`ExternalSync` invariant is preserved. Re-exporting `MemoryTaint` from the crate does
not weaken provenance. A dedicated seam test (W2) pins this: unknown db string → `ExternalSync`,
`Default::default()` → `Internal`, and round-trip of both db strings.

### `sqlite_conn()` escape hatch (W2 sub-decision)

The host `Memory` trait's `sqlite_conn()` (gap G1) is **not** part of the re-exported crate trait.
Keep it as a **host-side extension trait** during the transition; migrate internal raw-SQL callers to
`tinycortex::memory::chunks::with_connection` in W3; drive the residual count to zero in the deletion
ledger. Re-export covers the data types (`MemoryEntry`, `MemoryCategory`, `MemoryTaint`, `RecallOpts`,
`NamespaceSummary`, `GraphRelationRecord`, `RetrievalScoreBreakdown`, `NamespaceMemoryHit`, …) and the
`Memory` trait's async CRUD surface; the escape hatch stays host until W3 retires it.

---

## 0.4 — Toolchain baseline (result)

| Check | Result |
| --- | --- |
| Crate edition | 2021 (matches host) ✅ |
| `[patch.crates-io] tinycortex = { path = "vendor/tinycortex" }` | pre-staged in **both** worlds (root + `app/src-tauri`) ✅ |
| CI submodule checkout | recursive on all build/test lanes; covers `vendor/tinycortex` ✅ (verify release lanes in W1, as tinyagents needed) |
| **rusqlite alignment** | ✅ **resolved & merged (#59).** Crate was pinned `0.32` (bundled), host pins `=0.40.0` (bundled). Two `links = "sqlite3"` = hard Cargo error. Fixed in #59 (bump to `0.40` + `usize`→`i64`/`try_from` — the same sweep that closed drift **D3**). |
| **git2 alignment** | ✅ **resolved & merged (#59).** Crate was pinned `0.19`, host `0.21` (vendored-libgit2). Two `links = "git2"`. Fixed in #59 (bump to `0.21` + API deltas: `Tag::message`, `StringArray::Iter`, `Buf::as_str`). |
| Crate compiles with aligned deps | ✅ `cargo check --all-targets` clean; 38 diff/checkpoint tests pass. |
| **Host root world compiles with dep active** | ✅ `cargo check --manifest-path Cargo.toml --lib` **exit 0** with `tinycortex = "0.1"` active (`Cargo.toml:82`) + submodule at `33dda94`. **No `multiple packages link to native library` error** — one bundled SQLite + one libgit2 confirmed. **Now landed** (post-#59-merge): the `[dependencies]` line and gitlink are committed on the working branch, not reverted. Re-verified 2026-07-09. |
| Host `app/src-tauri` world | to verify in W1 (separate Cargo world / lockfile). |
| `GGML_NATIVE=OFF` macOS ARM | to verify on a macOS runner in W1 (no macOS host here). |

**Activation landed (post-#59).** The native-dep alignment merged upstream as **#59** and the host
gitlink was bumped to `33dda94`, so — per the submodule rule (bump only to a **merged** SHA) — W1's
activation is now in place: `[dependencies] tinycortex = "0.1"` is active in the root world
(`Cargo.toml:82`), the seam (`src/openhuman/tinycortex/`) is wired (`src/openhuman/mod.rs:130`), and
`cargo check --lib` is **exit 0**. Remaining §0.4 follow-ups: the `app/src-tauri` world and the
`GGML_NATIVE=OFF` macOS-runner check still verify in W1 (see rows above).

---

## 1. Ownership split (canonical, refined by the audits)

### Moves to TinyCortex (delete from host after cutover)

| Host module | Crate counterpart | Substrate tables that move |
| --- | --- | --- |
| `memory_store/{chunks,content(core),vectors,kv,entity_index,safety}` | `store/`, `chunks/` | `mem_tree_chunks`, `mem_tree_chunk_embeddings(+reembed_skipped)`, `vectors`, `kv_global`, `kv_namespace`, `store_meta`, `mem_tree_entity_index`, `mem_tree_entity_edges`, `mem_tree_entity_hotness`, `mem_tree_ingested_sources`, `mcp_writes`, `legacy_marker` |
| `memory_tree/{tree,retrieval,score,summarise}` | `tree/`, `retrieval/`, `score/` | `mem_tree_trees`, `mem_tree_summaries(+embeddings,+reembed_skipped)`, `mem_tree_buffers`, `mem_tree_score` |
| `memory_queue/` | `queue/` | `mem_tree_jobs` |
| `memory/ingest_pipeline.rs` internals | `ingest/` | — |
| `memory_diff`, `memory_entities`, `memory_graph`(engine), `memory_goals`, `memory_archivist`, `memory_sources`(registry + local readers), `memory_tools`(engine), `memory_conversations`(engine), `memory_search`(`vector`,`scoring`) | same-named crate modules | — |
| `memory/traits.rs` core types | `tinycortex::memory::{…}` (re-export) | — |

### Stays in OpenHuman (product policy, I/O, surfaces)

- **RPC surfaces:** `memory/{ops,schemas,schema,read_rpc}`, `rpc_models.rs`. Method names/payloads unchanged.
- **Agent tools:** `memory/tools/`, `memory/query/`, `memory_search/tools/`, `memory_tools`(tool surface) — thin wrappers over crate retrieval + `SecurityPolicy` gating.
- **Live sync (amended 2026-07-09, plan §8 / W-SYNC):** the sync **engine** (pipelines, per-toolkit
  Composio providers + HTTP client, canonicalize, sync_state, audit/rebuild, sync_status query,
  dispatcher) **moves to the crate** behind an optional `sync` cargo feature. The crate's
  "never makes a network call" invariant becomes **feature-scoped**: the default build stays
  network-free; the `sync` feature adds an HTTP Composio client whose credentials are injected by
  the host. Host retains: scheduler loops (tick-driven crate, like `queue::run_once`),
  credentials/OAuth (keychain `composio-direct`), event-bus bridges via a new **`SyncEventSink`**
  seam trait, RPC wrappers (`memory/{ops,schemas}/sync.rs`, `memory_sources/rpc.rs`), and the
  UnifiedMemory writeback via a new **`SkillDocSink`** seam trait. MCP transport stays host.
- **Process glue:** `memory/global.rs` singleton + queue worker; `memory/source_scope.rs` task-locals; `memory/chat.rs`; embeddings provider wiring. *(Amended, plan §8 / W-EMB: the provider **implementations** in `src/openhuman/embeddings/` migrate upstream into `tinyagents::harness::embeddings` — trait gains `name`/`model_id`/`signature` byte-pinned to `provider={name};model={model};dims={dims}` (P10) — and tinycortex bridges `EmbeddingBackend` to that trait; the host keeps factory/config/RPC wiring only.)*
- **Policy/UX:** `preferences.rs`, `remember.rs`, `tree_policy.rs`, `util/redact.rs`, config mapping.
- **Host-retained `UnifiedMemory` namespace-document tier** (0.3 key finding) — the 10 tables that
  coexist in the shared DB but **do not move**: `memory_docs`, `graph_global`, `graph_namespace`,
  `episodic_log` (+ `episodic_fts` + triggers), `event_log` (+ `event_fts`, `event_embeddings`,
  triggers), `conversation_segments`, `segment_embeddings`, `vector_chunks`, `user_profile`.
  These live in `memory_store/unified/{init,fts5,events,segments,profile}.rs` and remain host — the
  crate is the **primitive substrate**, not a drop-in for the whole DB.
- **Content-store host surfaces the crate explicitly excludes:** `content::wiki_git`,
  `content::obsidian`, `content::obsidian_registry`.

### The adapter seam: `src/openhuman/tinycortex/` (W1, mirrors `src/openhuman/tinyagents/`)

**W1 seam files** (all against seam traits already present in the crate, §0.2):
`embeddings.rs` (`EmbeddingBackend`/`Embedder`), `chat.rs` (`ChatProvider`/`Summariser`×2/
`EntityExtractor`/`GoalsGenerator`), `queue_driver.rs` (`QueueDelegates` + tokio worker loop +
Sentry/bus), `config.rs` (`Config`→`MemoryConfig`), `sinks.rs` (`TreeJobSink`/`TreeLeafSink`/
`SnapshotItemSource`/`EntityOccurrenceIndex`), `bus.rs` (engine outcomes → `DomainEvent`),
`mod.rs` (facade re-exports + boundary doc). All 17 W1 seam traits confirmed present (§0.2).

**Later seam additions (amended 2026-07-09):**

- **W-EMB rebridge** — `embeddings.rs` is re-pointed from `openhuman::embeddings` to
  `tinyagents::harness::embeddings::EmbeddingModel` (crate bridges `EmbeddingBackend` to that trait,
  plan §8.2 / gap-audit roster). Seam file stays; its backing implementation moves upstream.
- **W-SYNC seam file** — `sync_sink.rs` adds the host adapters for **two new crate traits** landing
  with W-SYNC.1 (not among the 17 above; they do not exist in the crate at the Phase-0 SHA):
  `SyncEventSink` (→ `MemorySyncStage` bus events) and `SkillDocSink` (→
  `MemoryClient::store_skill_sync`, the host-retained namespace-document tier). See plan §8.1 and the
  gap-audit roster.

---

## 2. Deletion ledger (skeleton)

Every legacy engine file is deleted only when its module's **drift row is closed**, its **gaps are
resolved**, and the **golden-workspace parity harness is green** for its flip. Counts from the host
audit SHA.

| Legacy module | Files (test files) | Deletes in | Preconditions |
| --- | --- | --- | --- |
| `memory_store/` | 66 (11) | W3 | drift **D3** closed; gap **G1** (escape hatch) migrated to `with_connection`; parity P3/P5/P11/P12 green; `unified/` tier re-homed as host-retained (kept, not deleted) |
| `memory_tree/` | 65 (7) | W5 | gaps **G3** (seal-embed), **G6** (2× Summariser) resolved; `source_scope` allowlist re-verified; parity P7/P11 green; `health/` + `tree_policy.rs` kept host (G5) |
| `memory_queue/` | 10 (1) | W4 | drift **D2** closed (predicate upstreamed); job payload_json parity (P4/P9); host worker loop + Sentry/degraded wiring kept host |
| `memory_conversations/` | 7 (1) | W7 | drift **D1** closed; `bus.rs` kept host |
| `memory_diff/` | 7 (0) | W7 | git-ledger parity (P9) green |
| `memory_entities/` | 3 (0) | W7 | parity P8 green |
| `memory_graph/` | 3 (0) | W7 | gap **G2** resolved (derive-on-read parity vs host-retained `graph_*`) |
| `memory_goals/` | 7 (0) | W7 | seam `GoalsGenerator` wired |
| `memory_archivist/` | 6 (0) | W7 | `TreeLeafSink` seam wired |
| `memory_sources/` | 16 (0) | W7 / W-SYNC | registry + local readers move (W7); `sync.rs` dispatcher + `reconcile.rs` move in W-SYNC; `rpc.rs` kept host |
| `memory_sync/` (engine) | — | **W-SYNC.3** | drift **D4** closed; W6 landed (crate ingest live); mocked + live Composio test pair green; sync-status parity green; schedulers/bus/RPC/keychain kept host |
| `src/openhuman/embeddings/` (provider impls) | — | **W-EMB.3** | tinyagents provider port merged; signature parity (P10) green; `factory.rs`(thin)/`rpc.rs`/`schemas.rs`/catalog kept host |
| `memory_tools/` | 10 (1) | W7 | engine → `tool_memory/`; tool surface kept host |
| `memory_search/` | 8 (0) | W5 | `vector`/`scoring` → crate `retrieval`/`score`; `tools/` kept host |
| `memory/ingest_pipeline.rs` internals | (thin entry points kept) | W6 | `ingest_chat`/`ingest_document_with_scope` signatures unchanged; 11 call sites untouched |

**Kept host (never deleted):** `memory/{ops,schemas,schema,read_rpc,tools,query,tree_source,
ingestion,util}`, `memory/{global,source_scope,chat,sync,preferences,remember,tree_policy,rpc_models,
traits(→re-exports)}.rs`, `memory_sync/`'s host-retained shell only (schedulers `periodic.rs`,
`bus.rs` subscribers, RPC registration — the engine moves in W-SYNC, plan §8), `memory_store/unified/*` (the namespace-document
tier), `memory_store/content/{wiki_git,obsidian,obsidian_registry}`, `memory_tree/health/`, and the
new `src/openhuman/tinycortex/` seam.

## 3. Workstream order (one workstream ≈ one host PR)

W1 seam scaffolding → W2 types/trait re-export → W3 store+chunks → W4 queue → W5 tree+retrieval+score
→ W6 ingest → **W-SYNC** (sync engine + Composio client, plan §8) → W7 long tail → W8 test-port +
golden parity sweep + deletion-ledger close-out. **W-EMB** (embeddings inheritance from tinyagents,
plan §8.2) runs in parallel; W-EMB.2 must land before the W-SYNC.3 flip.

Each risky workstream is a sandwich (plan §4): (a) tinycortex PR(s) closing that module's drift/gap
ledger, (b) host `chore(vendor): bump tinycortex`, (c) host cutover PR (adapter flip + legacy
deletion + host-side tests in the same PR for the ≥80% diff-coverage gate).

## 4. Security review items (must have dedicated seam tests)

1. **`MemoryTaint` fail-closed to `ExternalSync`** — proven identical (0.5); pin with a W2 seam test.
2. **`source_scope` per-turn allowlist** — must survive the W5 retrieval cutover; the retrieval
   primitives (`query_source`/`query_topic`/`drill_down`) run inside the host's task-local scope, and
   a W5 seam test must assert an out-of-allowlist source is not returned.
3. **Composio credential handling (W-SYNC)** — the crate holds the key only as a redacted
   `SecretString` (`Debug`/`Display` masked, serialization skipped); production resolution stays in
   the host keychain (`composio-direct`), the `COMPOSIO_API_KEY` env fallback is test-only; a seam
   test asserts the key never appears in `MemoryConfig` `Debug` output or client error messages
   (401 path included).
4. **Sync taint provenance (W-SYNC)** — every crate-side sync ingest must stamp
   `MemoryTaint::ExternalSync`; the mocked Composio pipeline test pins this.
