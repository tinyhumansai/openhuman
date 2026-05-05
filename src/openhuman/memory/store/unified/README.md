# Unified memory store

SQLite-backed implementation of the memory store. One `UnifiedMemory` struct owns a WAL-mode connection plus the on-disk markdown sidecar tree and vector storage path; the rest of this directory adds capabilities to it via per-domain `impl` blocks.

## Files

- **`mod.rs`** — declares the `UnifiedMemory` struct (connection + paths + embedder) and wires the submodules.
- **`init.rs`** — constructor, `CREATE TABLE` bootstrap (docs, kv, graph, vector chunks, episodic FTS5, segments, events, profile), idempotent legacy-namespace migrations, plus path / namespace helpers (`sanitize_namespace`, `now_ts`, `namespace_dir`).
- **`documents.rs`** — `memory_docs` CRUD: `upsert_document` (chunks + embeds + writes markdown sidecar), `upsert_document_metadata_only` (light path), `list_documents`, `list_namespaces`, `delete_document`, `clear_namespace`.
- **`kv.rs`** — global and namespace-scoped get/set/delete/list against `kv_global` / `kv_namespace`.
- **`graph.rs`** — `graph_namespace` / `graph_global` upserts with attribute merging and evidence accumulation, plus namespace / global / cross-namespace queries and document-scoped relation removal.
- **`query.rs`** — hybrid retrieval. Combines graph relevance, vector similarity, keyword overlap, episodic signal and freshness; exposes `query_namespace_*` (with query) and `recall_namespace_*` (query-less) entry points used by `MemoryClient`.
- **`helpers.rs`** — shared utilities: f32-vector byte codecs, cosine similarity, markdown chunking, text/graph normalisation, JSON attribute merging, recency scoring.
- **`fts5.rs`** — FTS5 episodic memory (`episodic_log` + `episodic_fts`). `EpisodicEntry` plus `episodic_insert` / `episodic_search` / `episodic_session_entries` for the Archivist and `search_memory` tool.
- **`segments.rs`** — conversation segmentation (`conversation_segments`). Boundary detection (time gap, embedding drift, explicit markers, turn count), segment lifecycle (open → closed → summarised), and the `BoundaryConfig` knobs.
- **`events.rs`** — event extraction (`event_log` + `event_fts`). Stores typed atomic events (Fact / Decision / Commitment / Preference / Question / Foresight) extracted from closed segments via heuristic pattern matching.
- **`profile.rs`** — user profile facets (`user_profile`). Evidence-backed `FacetType` rows that accumulate across sessions; on conflict, evidence count is bumped and the value is overwritten only if confidence improves.
- **`*_tests.rs`** — module-local tests for documents, events, profile, query, segments.

## How it fits

`MemoryClient` (in `../client.rs`) and the `impl Memory for UnifiedMemory` in `../memory_trait.rs` are the only things that should hold a `UnifiedMemory` directly. The ingestion pipeline (`../../ingestion/`) calls `upsert_document` and `graph_upsert_namespace` after parsing; the agent harness reads via `query_namespace_*` and `recall_namespace_*`; the Archivist writes episodic turns via `fts5::episodic_insert` and segments / events / profile facets via the dedicated submodules.
