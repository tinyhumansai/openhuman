# Memory

Persistent knowledge layer. Owns the unified store (SQLite + FTS5 + vector embeddings + graph relations), document ingestion pipelines, namespace + KV operations, conversation history, and retrieval scoring. Does NOT own raw provider embedding APIs (`local_ai/`), agent prompt assembly (`agent/memory_loader.rs`), or per-channel ingestion adapters beyond the bundled Slack importer.

## Architecture

The module is organised in concentric layers — the contract on the
inside, the persistent backend around it, the ingestion + retrieval
pipelines on top, and the per-domain glue at the edge:

```text
                      ┌──────────────────────────────────────┐
                      │  conversations/   slack_ingestion/   │  per-domain plumbing
                      ├──────────────────────────────────────┤
                      │  tree/   (bucket-seal LLD pipeline)  │  new retrieval architecture
                      ├──────────────────────────────────────┤
                      │  ingestion/        (extract chunks)  │  document ingestion
                      ├──────────────────────────────────────┤
                      │  store/      (UnifiedMemory backend) │  SQLite + FTS5 + vectors
                      ├──────────────────────────────────────┤
                      │  traits.rs           (Memory trait)  │  contract
                      └──────────────────────────────────────┘
```

- **`traits.rs`** — `Memory`, `MemoryEntry`, `MemoryCategory`,
  `RecallOpts`. The backend-agnostic contract every store implements.
- **`store/`** — `UnifiedMemory` is the production backend (SQLite
  with FTS5 for keyword search, vector tables for embeddings, and
  graph tables for entity/relation triples) plus the `MemoryClient`
  handle used by the rest of the process.
- **`ingestion/`** — chunking + extraction pipeline (entities,
  relations, embeddings) and the background `IngestionQueue` worker.
- **`tree/`** — the new bucket-seal retrieval architecture from
  `docs/MEMORY_ARCHITECTURE_LLD.md`: `canonicalize` (normalise
  inputs), `chunker` and `content_store` (durable chunks),
  `score`/`retrieval` (ranking surface),
  `tree_source`/`tree_topic`/`tree_global` (the three concentric
  trees the LLD calls for), and `jobs` (background seals/summaries).
- **`conversations/`** — workspace-backed JSONL chat thread/message
  history. See `conversations/README.md`.
- **`slack_ingestion/`** — Slack provider plumbing (bucketer +
  ingest wrapper + RPC). See `slack_ingestion/README.md`.

The legacy memory store (`store/` + `ingestion/`) and the new
`tree/` pipeline coexist for now — `tree/` is replacing the older
retrieval surface incrementally and both must remain wired into RPC
until the migration completes.

## Public surface

- `pub trait Memory` / `pub struct MemoryEntry` / `pub enum MemoryCategory` / `pub struct RecallOpts` — `traits.rs:11-100` — backend contract for any memory store.
- `pub struct UnifiedMemory` — `store/unified/` (re-exported `store/mod.rs:40`) — primary SQLite + FTS5 + vector implementation.
- `pub struct MemoryClient` / `pub struct MemoryClientRef` / `pub enum MemoryState` — `store/client.rs` — async client handle used by RPC handlers.
- `pub fn create_memory` / `pub fn create_memory_with_storage` / `pub fn create_memory_with_storage_and_routes` / `pub fn create_memory_for_migration` — `store/factories.rs` — bootstrap a memory instance.
- `pub struct MemoryIngestionRequest` / `pub struct MemoryIngestionResult` / `pub struct MemoryIngestionConfig` / `pub enum ExtractionMode` / `pub struct ExtractedEntity` / `pub struct ExtractedRelation` / `const DEFAULT_MEMORY_EXTRACTION_MODEL` — `ingestion.rs` (re-exported `mod.rs:22`).
- `pub struct IngestionQueue` / `pub struct IngestionJob` — `ingestion_queue.rs` — async background ingestion worker.
- `pub struct NamespaceDocumentInput` / `pub struct NamespaceMemoryHit` / `pub struct NamespaceQueryResult` / `pub struct NamespaceRetrievalContext` / `pub struct RetrievalScoreBreakdown` / `pub enum MemoryItemKind` — `store/types.rs`.
- RPC `memory.{init, list_documents, list_namespaces, delete_document, query_namespace, recall_context, recall_memories, list_files, read_file, write_file, namespace_list, doc_put, doc_ingest, doc_list, doc_delete, context_query, context_recall, kv_set, kv_get, kv_delete, kv_list_namespace, graph_upsert, graph_query, clear_namespace}` — `schemas.rs:29-55`.
- RPC tree `memory.tree.*` and retrieval — `tree/` (re-exported via `all_memory_tree_*` / `all_retrieval_*`).
- RPC slack ingestion — `slack_ingestion/` (re-exported via `all_slack_ingestion_*`).

## Calls into

- `src/openhuman/local_ai/` — embedding model, sentiment scoring, extraction LLM.
- `src/openhuman/embeddings/` — vector backend selection.
- `src/openhuman/config/` — memory backend choice + filesystem paths.
- `src/openhuman/encryption/` — at-rest secrets for KV namespaces.
- `src/core/event_bus/` — emits `DomainEvent::Memory(*)` on ingestion / mutation.

## Called by

- `src/openhuman/agent/` (`memory_loader.rs`, `harness/memory_context.rs`, `harness/archivist*.rs`, `harness/fork_context.rs`) — context injection and episodic indexing.
- `src/openhuman/learning/{reflection,tool_tracker,user_profile,prompt_sections}.rs` — long-term insight storage.
- `src/openhuman/screen_intelligence/{helpers,tests}.rs` — recall surfaces for visual context.
- `src/openhuman/autocomplete/history.rs` — query-history recall.
- `src/openhuman/tools/ops.rs` and `tools/impl/system/tool_stats.rs` — memory-backed tool stats.
- `src/core/all.rs` — registers `all_memory_*` controllers.

## Tests

- Unit: `ops_tests.rs`, `schemas_tests.rs`, `rpc_models_tests.rs`, `ingestion_tests.rs`, plus `*_tests.rs` files inside `store/`, `tree/`, `conversations/`, `slack_ingestion/`.
- Integration: `tests/autocomplete_memory_e2e.rs`, `tests/memory_graph_sync_e2e.rs`.
