# TinyCortex Memory Migration — Spec (Phase 0.5 / 0.6)

**Status:** Phase 0 baseline. Anchors the migration to exact reviewed SHAs and ties together the
drift, gap, and parity ledgers. Modeled on `docs/tinyagents-migration-spec.md` + its deletion ledger.

**Companion plan:** [`tinycortex-memory-migration-plan.md`](tinycortex-memory-migration-plan.md)
**Ledgers:** [`tinycortex-drift-ledger.md`](tinycortex-drift-ledger.md) ·
[`tinycortex-api-gap-audit.md`](tinycortex-api-gap-audit.md) ·
[`tinycortex-parity-checklist.md`](tinycortex-parity-checklist.md)

## Version anchors

| Repo | SHA | Note |
| --- | --- | --- |
| `tinyhumansai/openhuman` | `7850cf363559bcbb7ba688cbc4fccdb6bd9ce754` | host audit base (`main`, 2026-07-04) |
| `tinyhumansai/tinycortex` | `d1a8c7be2babc8fff7a72ed93861f459f3d6fa58` | crate audit base (v0.1.1) |
| `tinyhumansai/tinycortex` | `35f9518` (branch `chore/align-native-deps-for-openhuman`) | **first required upstream PR** — native-dep alignment (§0.4); host may not activate the dep until this merges |

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
| **rusqlite alignment** | ⚠️ **blocker resolved upstream.** Crate pinned `0.32` (bundled), host pins `=0.40.0` (bundled). Two `links = "sqlite3"` = hard Cargo error. Fixed in the crate PR (bump to `0.40` + `usize`→`i64`/`try_from`). |
| **git2 alignment** | ⚠️ **blocker resolved upstream.** Crate pinned `0.19`, host `0.21` (vendored-libgit2). Two `links = "git2"`. Fixed in the crate PR (bump to `0.21` + API deltas: `Tag::message`, `StringArray::Iter`, `Buf::as_str`). |
| Crate compiles with aligned deps | ✅ `cargo check --all-targets` clean; 38 diff/checkpoint tests pass. |
| **Host root world compiles with dep active** | ✅ `cargo check --manifest-path Cargo.toml --lib` **exit 0** with `tinycortex = "0.1"` active + submodule at `35f9518`. **No `multiple packages link to native library` error** — one bundled SQLite + one libgit2 confirmed. (Verified locally; the Cargo.toml/lock activation is reverted from the Phase-0 docs branch and re-lands in W1 post-upstream-merge.) |
| Host `app/src-tauri` world | to verify in W1 (separate Cargo world / lockfile). |
| `GGML_NATIVE=OFF` macOS ARM | to verify on a macOS runner in W1 (no macOS host here). |

**Activation is deferred to W1.** Per the submodule rule, the host may only bump the gitlink to a
**merged** upstream SHA. The native-dep alignment is committed on the crate branch and ready to PR;
once it merges and is published, W1 lands: (a) `chore(vendor): bump tinycortex`, (b)
`[dependencies] tinycortex = "0.1"` in both worlds. The `[dependencies]` line and the compile
verification below were validated **locally** against the branch to prove the baseline is sound.

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
- **Live sync:** all of `memory_sync/`, `memory/sync.rs` lifecycle + bus stage events.
- **Process glue:** `memory/global.rs` singleton + queue worker; `memory/source_scope.rs` task-locals; `memory/chat.rs`; embeddings provider wiring.
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

`embeddings.rs` (`EmbeddingBackend`/`Embedder`), `chat.rs` (`ChatProvider`/`Summariser`×2/
`EntityExtractor`/`GoalsGenerator`), `queue_driver.rs` (`QueueDelegates` + tokio worker loop +
Sentry/bus), `config.rs` (`Config`→`MemoryConfig`), `sinks.rs` (`TreeJobSink`/`TreeLeafSink`/
`SnapshotItemSource`/`EntityOccurrenceIndex`), `bus.rs` (engine outcomes → `DomainEvent`),
`mod.rs` (facade re-exports + boundary doc). All 17 seam traits confirmed present (§0.2).

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
| `memory_sources/` | 16 (0) | W7 | registry + local readers move; **live sync stays host** |
| `memory_tools/` | 10 (1) | W7 | engine → `tool_memory/`; tool surface kept host |
| `memory_search/` | 8 (0) | W5 | `vector`/`scoring` → crate `retrieval`/`score`; `tools/` kept host |
| `memory/ingest_pipeline.rs` internals | (thin entry points kept) | W6 | `ingest_chat`/`ingest_document_with_scope` signatures unchanged; 11 call sites untouched |

**Kept host (never deleted):** `memory/{ops,schemas,schema,read_rpc,tools,query,tree_source,
ingestion,util}`, `memory/{global,source_scope,chat,sync,preferences,remember,tree_policy,rpc_models,
traits(→re-exports)}.rs`, all of `memory_sync/`, `memory_store/unified/*` (the namespace-document
tier), `memory_store/content/{wiki_git,obsidian,obsidian_registry}`, `memory_tree/health/`, and the
new `src/openhuman/tinycortex/` seam.

## 3. Workstream order (one workstream ≈ one host PR)

W1 seam scaffolding → W2 types/trait re-export → W3 store+chunks → W4 queue → W5 tree+retrieval+score
→ W6 ingest → W7 long tail → W8 test-port + golden parity sweep + deletion-ledger close-out.

Each risky workstream is a sandwich (plan §4): (a) tinycortex PR(s) closing that module's drift/gap
ledger, (b) host `chore(vendor): bump tinycortex`, (c) host cutover PR (adapter flip + legacy
deletion + host-side tests in the same PR for the ≥80% diff-coverage gate).

## 4. Security review items (must have dedicated seam tests)

1. **`MemoryTaint` fail-closed to `ExternalSync`** — proven identical (0.5); pin with a W2 seam test.
2. **`source_scope` per-turn allowlist** — must survive the W5 retrieval cutover; the retrieval
   primitives (`query_source`/`query_topic`/`drill_down`) run inside the host's task-local scope, and
   a W5 seam test must assert an out-of-allowlist source is not returned.
