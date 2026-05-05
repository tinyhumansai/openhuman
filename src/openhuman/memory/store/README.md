# Memory store

Storage backend for the memory subsystem. Houses the SQLite + FTS5 + vector + graph implementation (`UnifiedMemory`), the async client handle used by RPC controllers (`MemoryClient`), the `Memory` trait impl bridging both, and the factory functions used to bootstrap a memory instance.

## Files

- **`mod.rs`** — module root; re-exports `UnifiedMemory`, `MemoryClient`, factory functions, and the public types from `types.rs`.
- **`types.rs`** — public input/output structs (`NamespaceDocumentInput`, `NamespaceMemoryHit`, `NamespaceRetrievalContext`, `RetrievalScoreBreakdown`, `MemoryItemKind`, `StoredMemoryDocument`, `MemoryKvRecord`, `GraphRelationRecord`) plus the `GLOBAL_NAMESPACE` sentinel.
- **`client.rs`** — `MemoryClient` / `MemoryClientRef` / `MemoryState`. Async wrapper around `UnifiedMemory` that owns the singleton ingestion queue and exposes the surface called by RPC handlers (`put_doc`, `ingest_doc`, `query_namespace`, `recall_namespace_*`, `kv_*`, `graph_*`, skill-sync helpers). Always local — no remote sync.
- **`client_tests.rs`** — coverage for the client-facing storage and graph round-trips against a fresh temp workspace.
- **`factories.rs`** — `create_memory*` constructors that select the embedding provider from `MemoryConfig` and instantiate `UnifiedMemory`. `effective_memory_backend_name` always reports `"namespace"`.
- **`memory_trait.rs`** — `impl Memory for UnifiedMemory`, mapping the generic trait surface (`store`, `recall`, `get`, `list`, `forget`, `namespace_summaries`) onto the unified store. Includes namespace normalisation and episodic-session augmentation.
- **`../safety/`** — shared secret-detection + redaction helpers used by memory write paths (documents, KV, episodic) to prevent credentials/tokens from being persisted into long-lived memory.
- **`unified/`** — the SQLite implementation, broken into per-table submodules. See `unified/README.md`.

## How it fits

Callers (RPC controllers, the agent harness, learning pipelines) interact with `MemoryClient`. The client delegates persistence to `UnifiedMemory` and offloads heavier work (chunk + embed + graph extraction) to the singleton `IngestionQueue` defined in `../ingestion/`. Generic consumers that just need `Memory` trait behaviour go through `memory_trait.rs`.
