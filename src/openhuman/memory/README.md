# memory

Orchestration layer over the memory stack. Owns:

- **Ingest pipeline** — orchestrates source → canonicalise → chunk →
  score → persist → enqueue extract jobs.
- **Job handlers** — background workers that drain the queue (admit,
  extract, seal, digest, topic-curate).
- **Scoring** — fast scorers, signal aggregation, score persistence.
- **Tree policy** — `tree_global` and `tree_topic` building rules.
- **RPC surface** — `read_rpc`, `tree_rpc`, controller schemas for the
  memory_* RPC namespace.
- **Recall** — `stm_recall`, `retrieval` ranking, query orchestration.

Does **not** own any storage primitives — those live in
[`memory_store`](../memory_store/). See that module for raw md, chunks,
entities, trees, vectors, kv, and contacts.

## Sibling memory_* modules

The memory stack is split across several top-level modules so each has
one job. memory orchestrates and routes between them.

| Module | Role |
| --- | --- |
| [`memory_store`](../memory_store/)         | Storage primitives: raw / chunks / entities / trees / vectors / kv / contacts. SQLite + on-disk md. |
| [`memory_tree`](../memory_tree/)           | Generic tree mechanics: bucket-seal, flush, summarise, walk. Kind-agnostic. |
| [`memory_archivist`](../memory_archivist/) | Chat conversation → clip tool-calls → push to tree. |
| [`memory_entities`](../memory_entities/)   | Md-backed entity registry (people + orgs + topics + …). Replacing `people/`. |
| [`memory_graph`](../memory_graph/)         | Derived co-occurrence edges over the entity index. |
| [`memory_tools`](../memory_tools/)         | Tool-scoped rules + agent read/write tools. |
| [`memory_sync`](../memory_sync/)           | Composio + workspace + MCP sync pipelines. |

## What lives here

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Module root + re-exports. |
| [`ingest_pipeline.rs`](ingest_pipeline.rs) | Source-agnostic ingest orchestration. Called by sync pipelines and tree_rpc. |
| [`jobs/`](../memory_queue/) | Background workers: extract, admit, seal, digest, topic curate. Re-exported here from `memory_queue`. |
| [`score/`](score/) | Fast scorer, signals, embeddings, entity extraction, entity-index persistence. `store.rs` will eventually split — entity-index pieces move to `memory_store::entities`. |
| [`retrieval/`](retrieval/) | Drill-down, fetch, query_source, query_global, query_topic, search; scoring + ranking on top of memory_store. |
| [`tree_global/`](../memory_tree/global/) | Global digest tree building policy: seal, digest, recap. Implemented in `memory_tree/global` and routed from here. |
| [`tree_topic/`](../memory_tree/topic/) | Topic tree building policy: hotness gating, routing, curator, backfill. Implemented in `memory_tree/topic` and routed from here. |
| [`summarizer/`](../memory_tree/summarise.rs) | LLM summarisation pipeline for sealed buckets. Implemented in `memory_tree/summarise.rs`. |
| [`stm_recall/`](stm_recall/) | Short-term recall: cross-session FTS5 lookup + ranking. |
| [`ingestion/`](ingestion/) | Document ingestion queue + extraction (entities, relations, embeddings) — feeds UnifiedMemory documents. |
| [`canonicalize/`](../memory_sync/canonicalize/) | Source → canonical markdown (chat / email / document). Implemented in `memory_sync/canonicalize` and used at ingest time. |
| [`chat/`](chat.rs) | Chat-source canonicalisation helpers. |
| [`conversations/`](../memory_conversations/) | Workspace-backed JSONL chat thread/message history. Re-exported here from `memory_conversations`. |
| [`read_rpc.rs`](read_rpc.rs) | RPC handlers for memory reads. |
| [`tree_rpc.rs`](tree_rpc.rs) | RPC handlers for tree ingest + introspection. |
| [`schemas/`](schemas/) + [`schema.rs`](schema.rs) | Controller schema definitions for the memory + memory_tree RPC namespaces. |
| [`sync_status/`](../memory_sync/sync_status/) | Sync freshness tracking + RPC. Re-exported here from `memory_sync::sync_status`. |
| [`ops/`](ops/) | RPC operation handlers + the shared `active_memory_client` helper. |
| [`preferences.rs`](preferences.rs) | User preference read/write helpers. |
| [`rpc_models.rs`](rpc_models.rs) | Shared RPC request/response shapes. |
| [`traits.rs`](traits.rs) | `Memory`, `MemoryEntry`, `MemoryCategory`, `NamespaceSummary`, `RecallOpts`. The backend-agnostic contract every store implements. |
| [`util/`](util/) | Small helpers (redact for log PII). |
| [`global.rs`](global.rs) | Global-namespace helpers. |

## Layer rules

- **No storage in this module.** All persistence goes through
  `memory_store::*`. If you're tempted to open a SQLite connection
  here, the connection helper belongs one layer down.
- **No upward dependencies.** memory may import from memory_store /
  memory_tree / memory_entities / memory_archivist / memory_graph /
  memory_tools, but the inverse is a layer violation. (The two
  documented exceptions today — `memory_store::retrieval::tree_walk`
  calling `memory::retrieval::drill_down`, and `memory_store::trees::registry`
  pulling `GLOBAL_SCOPE` from `memory::tree_global` — are tracked in
  their respective READMEs.)
- **Surface high-level tool calls** that route to the right submodule;
  don't expose internals at the call site.
