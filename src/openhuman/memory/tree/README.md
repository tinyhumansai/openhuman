# Memory tree

Bucket-seal-ready local memory architecture (Phase 1 of issue #707; the LLD design doc `docs/MEMORY_ARCHITECTURE_LLD.md` is referenced by the in-tree module headers but is not checked into this repo). Coexists with the legacy `store/` backend until full replacement.

## Pipeline

```text
source adapters (chat / email / document)
        │
        ▼
canonicalize/  ── normalised Markdown + provenance Metadata
        │
        ▼
chunker.rs    ── deterministic IDs, ≤3k-token bounded segments
        │
        ▼
content_store/── atomic .md files on disk (body + tags)
        │
        ▼
store.rs      ── SQLite persistence (chunks, scores, summaries, jobs, hotness)
        │
        ▼
score/        ── signals + embeddings + entity extraction
        │
        ▼
tree_source/  tree_topic/  tree_global/   ── per-scope summary trees
        │
        ▼
retrieval/    ── search / drill_down / topic / global / fetch
        │
        ▼
jobs/         ── background workers + scheduler (extract, admit, seal, digest)
```

## Files at this level

- [`mod.rs`](mod.rs) — Phase 1 module banner; re-exports controller registries (`all_memory_tree_*`, `all_retrieval_*`).
- [`chunker.rs`](chunker.rs) — slice canonical Markdown into ≤`DEFAULT_CHUNK_MAX_TOKENS` chunks; chat/email split at message boundaries, document at paragraphs.
- [`ingest.rs`](ingest.rs) — orchestrator: `canonicalize -> chunk -> stage_chunks -> fast score -> persist -> enqueue extract jobs`. Hot path; heavy work runs out of `jobs/`.
- [`rpc.rs`](rpc.rs) — JSON-RPC handlers for `memory_tree_ingest`, `list_chunks`, `get_chunk`, `trigger_digest`. Delegates to `ingest`/`store`/`jobs`.
- [`schemas.rs`](schemas.rs) — `ControllerSchema` definitions + `RegisteredController` wiring for the four `memory_tree_*` RPC methods.
- [`store.rs`](store.rs) — SQLite schema (chunks, score, entity index, trees, summaries, buffers, hotness, jobs) and accessors. Lazily initialised at `<workspace>/memory_tree/chunks.db`.
- [`store_tests.rs`](store_tests.rs) — store-layer unit tests.
- [`types.rs`](types.rs) — `Chunk`, `Metadata`, `SourceKind`, `DataSource`, `SourceRef`; deterministic `chunk_id` hash; `approx_token_count` heuristic.

## Subdirectories

- [`canonicalize/`](canonicalize/README.md) — chat / email / document → canonical Markdown + email body cleaner.
- [`chunker.rs`](chunker.rs) — see above.
- [`content_store/`](content_store/README.md) — on-disk `.md` files (atomic writes, paths, YAML compose, read+verify, tag rewrites).
- [`jobs/`](jobs/) — async job queue (extract / admit / seal / topic / digest workers).
- [`retrieval/`](retrieval/) — search and drill-down RPC surface.
- [`score/`](score/) — fast scorer, embeddings, entity extraction, score persistence.
- [`tree_source/`](tree_source/) — per-source summary trees (L0 buffer → L1 seal → cascade).
- [`tree_topic/`](tree_topic/) — per-entity topic trees, materialised lazily by hotness.
- [`tree_global/`](tree_global/) — daily global digest tree.
- [`util/`](util/README.md) — shared helpers (`redact` for log PII).
