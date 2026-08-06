# Namespace memory store

Host-retained SQLite namespace/document tier. One `UnifiedMemory` struct owns
the shared connection plus the on-disk markdown sidecar and compatibility
embedding handle; the rest of this directory adds the product-owned document,
graph, episodic, event, segment, profile, and retrieval policy via `impl`
blocks. TinyCortex owns the generic chunk/vector/tree/queue engine beside this
tier; this directory is intentionally not migration staging.

## Files

- **`mod.rs`** — declares the `UnifiedMemory` struct (connection + paths + embedder) and wires the submodules.
- **`init.rs`** — constructor, `CREATE TABLE` bootstrap (docs, kv, graph, vector chunks, episodic FTS5, segments, events, profile), idempotent legacy-namespace migrations, plus path / namespace helpers (`sanitize_namespace`, `now_ts`, `namespace_dir`).
- **`documents.rs`** — `memory_docs` CRUD: `upsert_document` (chunks + embeds + writes markdown sidecar), `upsert_document_metadata_only` (light path), `list_documents`, `list_namespaces`, `delete_document`, `clear_namespace`.
- **`kv.rs`** — global and namespace-scoped get/set/delete/list against `kv_global` / `kv_namespace`.
- **`../../safety/`** — secret redaction/validation helpers. Document, KV, and episodic writes sanitize credentials before persistence and emit `[memory:safety]` diagnostics when a payload is rewritten.

### Identifier canonicalization (namespace / key)

Content and identifiers are scrubbed by **different** rules, and mixing them up
caused #5164. `safety::canonical_identifier` (namespace, KV key) and
`safety::canonical_document_key` (document key) are the single source of truth:

- **Strict gating.** Only formatted / keyword-gated national IDs are rewritten
  (`has_likely_pii`). The lenient content scrubber (`redact_pii` on its own,
  used for titles/bodies/metadata) also rewrites bare digit runs, which the
  scanners legitimately use as identifiers — WhatsApp JIDs, iMessage `+1…` chat
  ids, timestamps, padded counters. Rewriting those maps two contacts onto one
  `(namespace, key)`, and the upsert's `ON CONFLICT … DO UPDATE` then has one
  contact's document overwrite the other's.
- **Symmetry.** An identifier is a storage *address*, so every path that
  addresses a row canonicalizes the same way: `sanitize_namespace` (`init.rs`)
  carries the namespace step for writes, reads, `query.rs`, `graph.rs`, deletes
  and the on-disk `namespaces/<ns>/` directory, and the by-key paths
  (`upsert_document*`, `Memory::get`, `Memory::forget`, the `kv.rs` shim) go
  through `canonical_document_key` / `canonical_identifier`. A read that skips
  the transform silently misses the row the write created, so the caller writes
  again — the unthrottled loop #5164 was reported for.
- **Never reject.** Rejecting the write instead returns an `Err` on every retry,
  which is what flooded Sentry (3,055 events / 1 user / 1 day). The rejections
  that remain deliberate (secret-shaped identifiers, empty keys) are demoted out
  of the error stream by `ExpectedErrorKind::MemoryIdentifierRejected`.
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
