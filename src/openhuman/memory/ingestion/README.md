# Memory ingestion

Pipeline that turns raw document text into chunks plus extracted entities and relations, then upserts everything into `UnifiedMemory`. Runs synchronously when callers need the result (`MemoryClient::ingest_doc`) and as a background worker for fire-and-forget submissions (`MemoryClient::put_doc`).

## Files

- **`mod.rs`** — adds `ingest_document` and `extract_graph` to `UnifiedMemory`, plus the internal `upsert_graph_relations` helper. Re-exports the public types and the queue / state surface.
- **`types.rs`** — public ingestion API: `MemoryIngestionRequest` / `MemoryIngestionResult` / `MemoryIngestionConfig`, `ExtractionMode` (sentence vs chunk), `ExtractedEntity` / `ExtractedRelation`, `DEFAULT_MEMORY_EXTRACTION_MODEL`. Crate-internal intermediates (`RawEntity`, `RawRelation`, `ExtractionUnit`, `ExtractionAccumulator`, `ParsedIngestion`) live here too.
- **`parse.rs`** — `parse_document` pipeline: chunking, header / metadata enrichment, alias resolution, regex- and rule-driven extraction. Produces a `ParsedIngestion`.
- **`regex.rs`** — lazily-initialised regexes (email headers, named emails, graph facts, ownership, preferences, action items, recipients, spatial relations, dates, person names) plus `sanitize_entity_name`, `sanitize_fact_text`, `classify_entity`.
- **`rules.rs`** — semantic validation rules for graph predicates (allowed head/tail entity types) and the `ExtractionAccumulator` impl that gates `add_entity` / `add_relation` on those rules.
- **`queue.rs`** — `IngestionQueue` (cloneable submit handle) plus `IngestionJob` and the background worker started via `start_worker_with_state`. The worker shares an `IngestionState` with synchronous callers so all ingestion serialises through the same singleton lock.
- **`state.rs`** — `IngestionState` / `IngestionStatusSnapshot`: queue depth, in-flight metadata, last-completed status, and the `tokio::sync::Mutex` that enforces single-threaded extraction (the local LLM path can't be re-entered safely).
- **`tests.rs`** — pipeline coverage exercising `parse_document`, regex extraction, and `UnifiedMemory::ingest_document` end-to-end.

## How it fits

`MemoryClient` owns the singleton `IngestionQueue` and forwards to it from `put_doc` (background) or `ingest_doc` (synchronous, behind the same lock). Every ingestion run publishes `MemoryIngestionStarted` / `MemoryIngestionCompleted` events on the global event bus so the UI status pill and `openhuman.memory_ingestion_status` RPC stay in sync. Output rows feed `UnifiedMemory`'s `memory_docs`, `vector_chunks`, and `graph_namespace` tables.
