# memory_store

Single home for every persisted memory shape. Owns the storage primitives —
nothing above this module touches SQLite or the on-disk vault directly.

```text
content/   on-disk .md files — SOURCE OF TRUTH for every body
chunks/    SQLite chunk rows (metadata + tags + md path pointer +
           lifecycle status) + the two chunkers that produce them
entities/  mem_tree_entity_index — every entity occurrence per node
trees/     summary tree persistence (one table, kind-parameterized)
vectors/   local vector DB (cosine, brute-force)
kv/        global + namespace key-value (kv_global, kv_namespace)
contacts/  [removed] facade over people::store (Person/Handle/Interaction)
unified/   [staging for removal] UnifiedMemory's remaining SQLite surface
           (documents/query/segments/events/profile) — replaced as the
           Memory trait callers migrate to per-kind backends
```

## Cross-cutting modules

| Path                                 | Role                                                                                                                                                                                                           |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`mod.rs`](mod.rs)                   | Module root + public re-exports.                                                                                                                                                                               |
| [`README.md`](README.md)             | You are here.                                                                                                                                                                                                  |
| [`kinds.rs`](kinds.rs)               | `MemoryKind` enum — the authoritative catalog: Raw / Chunk / Entity / Tree / Vector / Kv / Contact — plus per-kind type aliases.                                                                               |
| [`traits.rs`](traits.rs)             | `VectorEmbeddable` + `ObsidianRepresentable` + `ObsidianFile`. Every stored kind implements both — the compiler enforces "everything in memory_store is vector and obsidian compatible".                       |
| [`types.rs`](types.rs)               | Shared serde types used across submodules: `NamespaceDocumentInput`, `NamespaceMemoryHit`, `NamespaceQueryResult`, `NamespaceRetrievalContext`, `RetrievalScoreBreakdown`, `MemoryItemKind`, `MemoryKvRecord`. |
| [`memory_trait.rs`](memory_trait.rs) | `impl Memory for UnifiedMemory` — bridges the generic `Memory` trait surface onto the unified store.                                                                                                           |
| [`client.rs`](client.rs)             | `MemoryClient` / `MemoryClientRef` / `MemoryState`. Async wrapper over `UnifiedMemory` used by RPC controllers; owns the singleton ingestion-queue handle.                                                     |
| [`factories.rs`](factories.rs)       | `create_memory*` constructors. Selects the embedding provider per the `MemoryConfig`, probes Ollama health, and builds a `Box<dyn Memory>` over `UnifiedMemory`.                                               |
| [`retrieval/`](retrieval/)           | `RetrievalFacade` — single import surface over the four retrieval modes (tree-walk, vector, keyword, param/tag).                                                                                               |
| [`tools/`](tools/)                   | Agent tools that read directly from memory_store: `memory_store_raw_search`, `memory_store_raw_chunks`, `memory_store_kinds`.                                                                                  |

## Storage submodules

| Path | Owns |
| --- | --- |
| [`content/`](content/) | **Source of truth** for chunk + summary bodies as on-disk `.md` files. Atomic writes, path layout, YAML front-matter compose/parse, tag rewrites, Obsidian vault defaults. See [`content/README.md`](content/README.md). |
| [`chunks/`](chunks/) | Full chunk lifecycle. `types.rs` (`Chunk`, `Metadata`, `SourceKind`, `RawRef`, `ListChunksQuery`) + `store.rs` (SQLite persistence + connection cache) + `produce.rs` (source-kind dispatch chunker used by the ingest pipeline) + `semantic.rs` (heading/paragraph-aware chunker). |
| [`entities.rs`](entities.rs) | Thin re-export of `memory_tree::score::store` — `index_entity`, `index_entities`, `lookup_entity`, `list_entity_ids_for_node`, `clear_entity_index_for_node`, `count_entity_index`, `EntityHit`. Reads/writes the `mem_tree_entity_index` table. |
| [`trees/`](trees/) | `store.rs` (`mem_tree_trees` / `mem_tree_summaries` / `mem_tree_buffers`), `types.rs` (Tree / SummaryNode / TreeKind / TreeStatus / Buffer + topic hotness types), `registry.rs` (kind-parameterized helpers), `hotness.rs` (entity hotness side-table). |
| [`vectors/`](vectors/) | Standalone vector store. `VectorStore` over SQLite, byte-codec for f32 vectors, cosine similarity. |
| [`kv.rs`](kv.rs) | Global + namespace key-value (`kv_global`, `kv_namespace` tables). |
| `contacts/` | Removed. Contact access now lives outside `memory_store` via `people::store`. |
| [`unified/`](unified/) | **Staging for removal.** Shrinking SQLite surface that still backs the `Memory` trait while callers migrate to per-kind modules. Active pieces today: `documents`, `query`, `segments`, `events`, `profile`. The `fts5` episodic surface is replaced by [`memory_archivist`](../memory_archivist/) and `graph` by [`memory_graph`](../memory_graph/). See [`unified/README.md`](unified/README.md). |

## Layer rules

- **Content bytes are immutable.** The `.md` file written by `content/` is
  the source of truth; SQLite stores a `(content_path, content_sha256)`
  pointer. The body never changes after the first write — only YAML
  front-matter (`tags:`) is rewritable.
- **SQLite is for indexing and vectors.** Anything keyword/param-searchable
  on the body itself should be served by grepping the `.md` files.
- **No upward dependencies.** memory_store does not depend on
  `memory_tree`, `memory_tools`, or `memory`. The one documented exception
  is `retrieval::RetrievalFacade::tree_walk`, which delegates to
  `memory_tree::retrieval::drill_down`; revisit when drill_down's policy bits
  can be cleanly separated from its pure traversal.
