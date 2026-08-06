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
[`memory_store`](store/). See that module for raw md, chunks,
entities, trees, vectors, kv, and contacts.

## Sibling memory\_\* modules

The memory stack is split across several top-level modules so each has
one job. memory orchestrates and routes between them.

| Module                                     | Role                                                                                                |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------- |
| [`memory_store`](store/)         | Storage primitives: raw / chunks / entities / trees / vectors / kv / contacts. SQLite + on-disk md. |
| [`memory_tree`](tree/)           | Generic tree mechanics: bucket-seal, flush, summarise, and retrieval/traversal backends.            |
| `tinycortex::memory::archivist`            | Chat conversation → clip tool-calls → push to tree; called directly by the host harness.            |
| [`tool_memory`](tool_memory/)         | Tool-scoped rules + agent read/write tools.                                                         |
| [`memory_sync`](sync/)           | Composio + workspace + MCP sync pipelines.                                                          |

## What lives here

| Path                                              | Role                                                                                                                                                                    |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`mod.rs`](mod.rs)                                | Module root + orchestration-facing exports.                                                                                                                              |
| [`sync_events.rs`](sync_events.rs)                              | High-level sync lifecycle types + frontend-visible stage events.                                                                                                         |
| [`query/`](query/)                                | High-level memory query tools, including the agentic tree-walk flow.                                                                                                     |
| [`remember.rs`](remember.rs)                      | High-level remember source classification (`chat_history`, `uploaded_data`, `llm_thought`).                                                                              |
| [`ingest_pipeline.rs`](ingest_pipeline.rs)        | Source-agnostic ingest orchestration. Called by sync pipelines and tree ingest RPC.                                                                                      |
| [`ingestion/`](ingestion/)                        | Document ingestion queue + extraction (entities, relations, embeddings) — feeds UnifiedMemory documents.                                                                |
| [`tinycortex::memory::ingest::canonicalize`](https://github.com/tinyhumansai/tinycortex/tree/main/src/memory/ingest/canonicalize) | Source → canonical markdown (chat / email / document), owned by TinyCortex and used at ingest time. |
| [`chat/`](chat.rs)                                | Chat-source canonicalisation helpers.                                                                                                                                   |
| [`read_rpc/`](read_rpc/)                           | RPC handlers for memory reads.                                                                                                                                          |
| [`schemas/`](schemas/) + [`schema/`](schema/)      | Controller schema definitions for the memory + memory_tree RPC namespaces.                                                                                              |
| [`sync_status/`](sync/sync_status/)     | Sync freshness tracking + RPC.                                                                                                                                          |
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
