# memory

Orchestration layer over the memory stack. Owns:

- **Sync orchestration** — accepts cron/manual sync requests, emits
  frontend-visible lifecycle events, and dispatches into `memory_sync`.
- **Query orchestration** — surfaces the high-level memory query tools
  and agentic tree walk flow, delegating traversal/retrieval to `memory_tree`.
- **Remember orchestration** — classifies chat history, uploaded data,
  and LLM-thought memory before routing it onward.
- **Ingest pipeline** — orchestrates source → canonicalise → chunk →
  score → persist → enqueue extract jobs.
- **RPC surface** — `read_rpc`, sync handlers, controller schemas for the
  memory\_\* RPC namespace.

Does **not** own any storage primitives — those live in
[`memory_store`](../memory_store/). See that module for raw md, chunks,
entities, trees, vectors, kv, and contacts.

## Sibling memory\_\* modules

The memory stack is split across several top-level modules so each has
one job. memory orchestrates and routes between them.

| Module                                     | Role                                                                                                |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------- |
| [`memory_store`](../memory_store/)         | Storage primitives: raw / chunks / entities / trees / vectors / kv / contacts. SQLite + on-disk md. |
| [`memory_tree`](../memory_tree/)           | Generic tree mechanics: bucket-seal, flush, summarise, and retrieval/traversal backends.            |
| [`memory_archivist`](../memory_archivist/) | Chat conversation → clip tool-calls → push to tree.                                                 |
| [`memory_entities`](../memory_entities/)   | Md-backed entity registry (people + orgs + topics + …). Replacing `people/`.                        |
| [`memory_graph`](../memory_graph/)         | Derived co-occurrence edges over the entity index.                                                  |
| [`memory_tools`](../memory_tools/)         | Tool-scoped rules + agent read/write tools.                                                         |
| [`memory_sync`](../memory_sync/)           | Composio + workspace + MCP sync pipelines.                                                          |

## What lives here

| Path                                              | Role                                                                                                                                                                    |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`mod.rs`](mod.rs)                                | Module root + orchestration-facing exports.                                                                                                                              |
| [`sync.rs`](sync.rs)                              | High-level sync lifecycle types + frontend-visible stage events.                                                                                                         |
| [`query/`](query/)                                | High-level memory query tools, including the agentic tree-walk flow.                                                                                                     |
| [`remember.rs`](remember.rs)                      | High-level remember source classification (`chat_history`, `uploaded_data`, `llm_thought`).                                                                              |
| [`ingest_pipeline.rs`](ingest_pipeline.rs)        | Source-agnostic ingest orchestration. Called by sync pipelines and tree ingest RPC.                                                                                      |
| [`ingestion/`](ingestion/)                        | Document ingestion queue + extraction (entities, relations, embeddings) — feeds UnifiedMemory documents.                                                                |
| [`canonicalize/`](../memory_sync/canonicalize/)   | Source → canonical markdown (chat / email / document). Implemented in `memory_sync/canonicalize` and used at ingest time.                                               |
| [`chat/`](chat.rs)                                | Chat-source canonicalisation helpers.                                                                                                                                   |
| [`read_rpc.rs`](read_rpc.rs)                      | RPC handlers for memory reads.                                                                                                                                          |
| [`schemas/`](schemas/) + [`schema.rs`](schema.rs) | Controller schema definitions for the memory + memory_tree RPC namespaces.                                                                                              |
| [`sync_status/`](../memory_sync/sync_status/)     | Sync freshness tracking + RPC.                                                                                                                                          |
| [`ops/`](ops/)                                    | RPC operation handlers + the shared `active_memory_client` helper.                                                                                                      |
| [`preferences.rs`](preferences.rs)                | User preference read/write helpers.                                                                                                                                     |
| [`rpc_models.rs`](rpc_models.rs)                  | Shared RPC request/response shapes.                                                                                                                                     |
| [`traits.rs`](traits.rs)                          | `Memory`, `MemoryEntry`, `MemoryCategory`, `NamespaceSummary`, `RecallOpts`. The backend-agnostic contract every store implements.                                      |
| [`util/`](util/)                                  | Small helpers (redact for log PII).                                                                                                                                     |
| [`global.rs`](global.rs)                          | Global-namespace helpers.                                                                                                                                               |

## Layer rules

- **No storage in this module.** All persistence goes through
  `memory_store::*`. If you're tempted to open a SQLite connection
  here, the connection helper belongs one layer down.
- **Orchestration lives here.** High-level sync/query/remember decisions
  should land in `memory`; sibling `memory_*` modules do the backend work.
- **Surface high-level tool calls** that route to the right submodule;
  don't expose internals at the call site.
