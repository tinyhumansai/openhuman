//! Controller schemas for the memory tree.
//!
//! Registered JSON-RPC methods include the original Phase 1 surface
//! (`ingest`, `list_chunks`, `get_chunk`, `trigger_digest`) plus the new
//! Memory-tab read RPCs added by the cloud-default backend refactor:
//! `list_sources`, `search`, `recall`, `entity_index_for`,
//! `top_entities`, `chunk_score`, `delete_chunk`, plus
//! `get_llm` / `set_llm` for the backend-selector UI.
//!
//! Handlers delegate to [`super::rpc`] (write side) or
//! [`super::read_rpc`] (UI read side).

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::read_rpc;
use crate::openhuman::memory::tree::rpc as tree_rpc;
use crate::rpc::RpcOutcome;

const NAMESPACE: &str = "memory_tree";

/// All `memory_tree` controller schemas, used by the registry to advertise
/// inputs/outputs to CLI + JSON-RPC consumers.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("ingest"),
        schemas("list_chunks"),
        schemas("get_chunk"),
        schemas("trigger_digest"),
        schemas("list_sources"),
        schemas("search"),
        schemas("recall"),
        schemas("entity_index_for"),
        schemas("chunks_for_entity"),
        schemas("top_entities"),
        schemas("chunk_score"),
        schemas("delete_chunk"),
        schemas("get_llm"),
        schemas("set_llm"),
        schemas("graph_export"),
        schemas("flush_now"),
        schemas("wipe_all"),
        schemas("reset_tree"),
    ]
}

/// Registered `memory_tree` controllers (schema + handler pairs) wired into
/// `core::all`.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("ingest"),
            handler: handle_ingest,
        },
        RegisteredController {
            schema: schemas("list_chunks"),
            handler: handle_list_chunks,
        },
        RegisteredController {
            schema: schemas("get_chunk"),
            handler: handle_get_chunk,
        },
        RegisteredController {
            schema: schemas("trigger_digest"),
            handler: handle_trigger_digest,
        },
        RegisteredController {
            schema: schemas("list_sources"),
            handler: handle_list_sources,
        },
        RegisteredController {
            schema: schemas("search"),
            handler: handle_search,
        },
        RegisteredController {
            schema: schemas("recall"),
            handler: handle_recall,
        },
        RegisteredController {
            schema: schemas("entity_index_for"),
            handler: handle_entity_index_for,
        },
        RegisteredController {
            schema: schemas("chunks_for_entity"),
            handler: handle_chunks_for_entity,
        },
        RegisteredController {
            schema: schemas("top_entities"),
            handler: handle_top_entities,
        },
        RegisteredController {
            schema: schemas("chunk_score"),
            handler: handle_chunk_score,
        },
        RegisteredController {
            schema: schemas("delete_chunk"),
            handler: handle_delete_chunk,
        },
        RegisteredController {
            schema: schemas("get_llm"),
            handler: handle_get_llm,
        },
        RegisteredController {
            schema: schemas("set_llm"),
            handler: handle_set_llm,
        },
        RegisteredController {
            schema: schemas("graph_export"),
            handler: handle_graph_export,
        },
        RegisteredController {
            schema: schemas("flush_now"),
            handler: handle_flush_now,
        },
        RegisteredController {
            schema: schemas("wipe_all"),
            handler: handle_wipe_all,
        },
        RegisteredController {
            schema: schemas("reset_tree"),
            handler: handle_reset_tree,
        },
    ]
}

/// Lookup the [`ControllerSchema`] for a single `memory_tree` function name.
pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "ingest" => ControllerSchema {
            namespace: NAMESPACE,
            function: "ingest",
            description: "Ingest a source into canonical chunks. \
                 Dispatches on `source_kind`; `payload` shape depends on the kind \
                 (chat → ChatBatch, email → EmailThread, document → DocumentInput).",
            inputs: vec![
                FieldSchema {
                    name: "source_kind",
                    ty: TypeSchema::Enum {
                        variants: vec!["chat", "email", "document"],
                    },
                    comment: "Which source kind the payload represents.",
                    required: true,
                },
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Stable logical source id (channel, thread, document id).",
                    required: true,
                },
                FieldSchema {
                    name: "owner",
                    ty: TypeSchema::String,
                    comment: "Optional account / user this content belongs to.",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tags or labels carried through.",
                    required: false,
                },
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Json,
                    comment: "Adapter-specific payload. \
                         chat: {platform, channel_label, messages[]}. \
                         email: {provider, thread_subject, messages[]}. \
                         document: {provider, title, body, modified_at, source_ref}.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Logical source id the ingest was scoped to.",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_written",
                    ty: TypeSchema::U64,
                    comment: "Number of chunks persisted after admission.",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_dropped",
                    ty: TypeSchema::U64,
                    comment: "Number of chunks rejected by the admission gate.",
                    required: true,
                },
                FieldSchema {
                    name: "chunk_ids",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "IDs of all chunks persisted after admission.",
                    required: true,
                },
            ],
        },
        "list_chunks" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_chunks",
            description:
                "Paginated list of chunks with optional filters by source kind / source id / \
                 entity ids / time window / keyword. Returns chunks plus total match count for \
                 pagination.",
            inputs: vec![
                FieldSchema {
                    name: "source_kinds",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Restrict to one or more source kinds (chat / email / document).",
                    required: false,
                },
                FieldSchema {
                    name: "source_ids",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Restrict to one or more logical source ids.",
                    required: false,
                },
                FieldSchema {
                    name: "entity_ids",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Restrict to chunks indexed against any of these canonical entity ids.",
                    required: false,
                },
                FieldSchema {
                    name: "since_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Inclusive lower bound on chunk timestamp (ms since epoch).",
                    required: false,
                },
                FieldSchema {
                    name: "until_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Inclusive upper bound on chunk timestamp (ms since epoch).",
                    required: false,
                },
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Substring keyword filter over chunk preview content.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum rows per page (defaults to 50, capped at 1000).",
                    required: false,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Pagination offset (defaults to 0).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "chunks",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Chunk"))),
                    comment: "Page of matching chunks ordered by timestamp DESC.",
                    required: true,
                },
                FieldSchema {
                    name: "total",
                    ty: TypeSchema::U64,
                    comment: "Total number of chunks matching the filter (pre-pagination).",
                    required: true,
                },
            ],
        },
        "get_chunk" => ControllerSchema {
            namespace: NAMESPACE,
            function: "get_chunk",
            description: "Fetch a single chunk by its deterministic id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Chunk id (32 hex chars).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "chunk",
                ty: TypeSchema::Option(Box::new(TypeSchema::Ref("Chunk"))),
                comment: "The chunk if found, otherwise null.",
                required: false,
            }],
        },
        "list_sources" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_sources",
            description:
                "Distinct (source_kind, source_id) pairs with chunk counts and most-recent timestamps. \
                 `display_name` is computed from the source_id (un-slug + strip user email when known).",
            inputs: vec![FieldSchema {
                name: "user_email_hint",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "When provided, source ids that contain this email get it stripped from \
                          their display name so the UI shows the other party of an email thread.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "sources",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Source"))),
                comment: "All distinct ingest sources, newest activity first.",
                required: true,
            }],
        },
        "search" => ControllerSchema {
            namespace: NAMESPACE,
            function: "search",
            description:
                "Keyword LIKE-search over chunk bodies. Cheap, deterministic; useful as a \
                 fallback when semantic recall is unavailable.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Substring to match against chunk content.",
                    required: true,
                },
                FieldSchema {
                    name: "k",
                    ty: TypeSchema::U64,
                    comment: "Maximum chunks to return.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "chunks",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Chunk"))),
                comment: "Matching chunks ordered by recency.",
                required: true,
            }],
        },
        "recall" => ControllerSchema {
            namespace: NAMESPACE,
            function: "recall",
            description:
                "Semantic recall — runs the Phase 4 cosine rerank against the query embedding \
                 and returns leaf chunks (not summaries) for UI display.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Free-text query — embedded once and reranked against summary embeddings.",
                    required: true,
                },
                FieldSchema {
                    name: "k",
                    ty: TypeSchema::U64,
                    comment: "Maximum chunks to return.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "chunks",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Chunk"))),
                    comment: "Recalled chunks, sorted in the same order as the rerank.",
                    required: true,
                },
                FieldSchema {
                    name: "scores",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                    comment: "Parallel array of similarity scores (one per chunk).",
                    required: true,
                },
            ],
        },
        "entity_index_for" => ControllerSchema {
            namespace: NAMESPACE,
            function: "entity_index_for",
            description: "Return all canonical entities indexed against a chunk (or summary node) id.",
            inputs: vec![FieldSchema {
                name: "chunk_id",
                ty: TypeSchema::String,
                comment: "Chunk id (32 hex chars).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "entities",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("EntityRef"))),
                comment: "Entities attached to the node, ordered by mention count DESC.",
                required: true,
            }],
        },
        "chunks_for_entity" => ControllerSchema {
            namespace: NAMESPACE,
            function: "chunks_for_entity",
            description:
                "Return chunk IDs that reference an entity_id (inverse of entity_index_for). \
                 Used by the Memory tab's People/Topics lenses to filter the chunk list.",
            inputs: vec![FieldSchema {
                name: "entity_id",
                ty: TypeSchema::String,
                comment:
                    "Canonical entity id (e.g. `person:Steven Enamakel`, \
                     `email:alice@example.com`).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "chunk_ids",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Chunk ids that mention the entity, ordered by recency DESC.",
                required: true,
            }],
        },
        "top_entities" => ControllerSchema {
            namespace: NAMESPACE,
            function: "top_entities",
            description:
                "Most-frequent canonical entities across the workspace, optionally narrowed by kind.",
            inputs: vec![
                FieldSchema {
                    name: "kind",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to a single entity_kind (`person`, `email`, `topic`, …).",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Maximum rows to return.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "entities",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("EntityRef"))),
                comment: "Top entities, ordered by mention count DESC.",
                required: true,
            }],
        },
        "chunk_score" => ControllerSchema {
            namespace: NAMESPACE,
            function: "chunk_score",
            description:
                "Score breakdown stored in `mem_tree_score` for one chunk — used by the Memory \
                 tab's 'why was this kept / dropped' panel.",
            inputs: vec![FieldSchema {
                name: "chunk_id",
                ty: TypeSchema::String,
                comment: "Chunk id (32 hex chars).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "breakdown",
                ty: TypeSchema::Option(Box::new(TypeSchema::Ref("ScoreBreakdown"))),
                comment: "Per-signal weight + value array, total, threshold, kept flag, llm_consulted flag.",
                required: false,
            }],
        },
        "delete_chunk" => ControllerSchema {
            namespace: NAMESPACE,
            function: "delete_chunk",
            description:
                "Purge one chunk plus its score row, entity-index rows, and on-disk .md file. \
                 Idempotent — missing chunk returns deleted=false. Does NOT cascade through \
                 sealed summaries; UIs warn the user.",
            inputs: vec![FieldSchema {
                name: "chunk_id",
                ty: TypeSchema::String,
                comment: "Chunk id to remove.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "deleted",
                    ty: TypeSchema::Bool,
                    comment: "True when the chunk row was found and removed.",
                    required: true,
                },
                FieldSchema {
                    name: "score_rows_removed",
                    ty: TypeSchema::U64,
                    comment: "Count of rows removed from `mem_tree_score`.",
                    required: true,
                },
                FieldSchema {
                    name: "entity_index_rows_removed",
                    ty: TypeSchema::U64,
                    comment: "Count of rows removed from `mem_tree_entity_index`.",
                    required: true,
                },
            ],
        },
        "get_llm" => ControllerSchema {
            namespace: NAMESPACE,
            function: "get_llm",
            description: "Read the currently configured LLM backend (`cloud` or `local`).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "current",
                ty: TypeSchema::Enum {
                    variants: vec!["cloud", "local"],
                },
                comment: "Active backend string.",
                required: true,
            }],
        },
        "set_llm" => ControllerSchema {
            namespace: NAMESPACE,
            function: "set_llm",
            description:
                "Update the LLM backend selector and (optionally) per-role model choices \
                 (`cloud_model`, `extract_model`, `summariser_model`) and persist the \
                 result to config.toml in a single atomic write. Absent model fields \
                 leave the corresponding config key unchanged so a caller flipping just \
                 the backend doesn't have to re-supply every model id.",
            inputs: vec![
                FieldSchema {
                    name: "backend",
                    ty: TypeSchema::Enum {
                        variants: vec!["cloud", "local"],
                    },
                    comment: "New backend value.",
                    required: true,
                },
                FieldSchema {
                    name: "cloud_model",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Cloud model id (used when backend=cloud). \
                              Absent → leave existing memory_tree.cloud_llm_model unchanged.",
                    required: false,
                },
                FieldSchema {
                    name: "extract_model",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Ollama model id for the entity extractor (used when backend=local). \
                              Absent → leave existing memory_tree.llm_extractor_model unchanged.",
                    required: false,
                },
                FieldSchema {
                    name: "summariser_model",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Ollama model id for the summariser (used when backend=local). \
                              Absent → leave existing memory_tree.llm_summariser_model unchanged.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "current",
                ty: TypeSchema::Enum {
                    variants: vec!["cloud", "local"],
                },
                comment: "The effective backend after the call.",
                required: true,
            }],
        },
        "wipe_all" => ControllerSchema {
            namespace: NAMESPACE,
            function: "wipe_all",
            description: "Destructive reset: truncate every mem_tree_* table, remove the \
                          on-disk content folders (raw / wiki / email / chat / document / \
                          legacy summaries) under the workspace memory_tree content root, \
                          and clear every Composio sync-state KV row so the next sync \
                          re-fetches all upstream items. Used by the Memory tab's 'Reset \
                          memory' button.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "rows_deleted",
                    ty: TypeSchema::U64,
                    comment: "Total mem_tree_* rows removed across all tables.",
                    required: true,
                },
                FieldSchema {
                    name: "dirs_removed",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Top-level directories under content_root that were deleted.",
                    required: true,
                },
                FieldSchema {
                    name: "sync_state_cleared",
                    ty: TypeSchema::U64,
                    comment: "Composio sync-state KV rows deleted (cursors + synced-id sets).",
                    required: true,
                },
            ],
        },
        "reset_tree" => ControllerSchema {
            namespace: NAMESPACE,
            function: "reset_tree",
            description: "Wipe summary-tree state but keep chunks + raw archive + sync state, \
                          then re-enqueue every chunk through the extraction pipeline so the \
                          tree rebuilds from scratch. Useful after changing the summariser \
                          backend (e.g. enabling a local LLM) without paying the upstream \
                          re-sync cost.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "tree_rows_deleted",
                    ty: TypeSchema::U64,
                    comment: "Tree-state rows removed (summaries + trees + buffers + jobs).",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_requeued",
                    ty: TypeSchema::U64,
                    comment: "Chunks reset to lifecycle_status = 'pending_extraction'.",
                    required: true,
                },
                FieldSchema {
                    name: "jobs_enqueued",
                    ty: TypeSchema::U64,
                    comment: "extract_chunk jobs enqueued (one per chunk).",
                    required: true,
                },
            ],
        },
        "flush_now" => ControllerSchema {
            namespace: NAMESPACE,
            function: "flush_now",
            description: "Manually trigger the summary-tree build. Enqueues a flush_stale \
                          job with max_age_secs=0 so every L0 buffer force-seals immediately; \
                          the seal worker runs each through the configured (cloud or local) \
                          summariser. Idempotent — same UTC-day dedupe key as the scheduled \
                          flush so spamming the button is safe.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "enqueued",
                    ty: TypeSchema::Bool,
                    comment: "True when a fresh job row was inserted; false when an active \
                              flush job already exists for today.",
                    required: true,
                },
                FieldSchema {
                    name: "stale_buffers",
                    ty: TypeSchema::U64,
                    comment: "Count of L0 buffers that currently qualify for force-seal.",
                    required: true,
                },
            ],
        },
        "graph_export" => ControllerSchema {
            namespace: NAMESPACE,
            function: "graph_export",
            description: "Return either the summary tree (parent→child links between sealed \
                          summary nodes) or the document↔contact graph (chunks linked to \
                          person entities they mention). Includes the absolute path to the \
                          on-disk content root so deep links can point Obsidian at the same \
                          files.",
            inputs: vec![FieldSchema {
                name: "mode",
                ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                    variants: vec!["tree", "contacts"],
                })),
                comment: "Which graph to return. Defaults to `tree`.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "nodes",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GraphNode"))),
                    comment: "Summary, chunk, or contact nodes depending on mode.",
                    required: true,
                },
                FieldSchema {
                    name: "edges",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GraphEdge"))),
                    comment: "Explicit edges. Empty in tree mode (parent_id encodes \
                              edges); chunk→contact mention edges in contacts mode.",
                    required: true,
                },
                FieldSchema {
                    name: "content_root_abs",
                    ty: TypeSchema::String,
                    comment: "Absolute path to <workspace>/memory_tree/content/.",
                    required: true,
                },
            ],
        },
        "trigger_digest" => ControllerSchema {
            namespace: NAMESPACE,
            function: "trigger_digest",
            description: "Manually enqueue a daily-digest job for the global \
                tree. Idempotent — re-running for a day that already has a \
                digest is a no-op (the handler skips). When no date is \
                supplied, defaults to yesterday in UTC, matching the \
                scheduler's autonomous behavior.",
            inputs: vec![FieldSchema {
                name: "date_iso",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "UTC calendar date as `YYYY-MM-DD`. Optional; \
                    defaults to yesterday when omitted.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "enqueued",
                    ty: TypeSchema::Bool,
                    comment: "True when a fresh job row was inserted; false \
                        when an active job for the same date suppressed it.",
                    required: true,
                },
                FieldSchema {
                    name: "job_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "ID of the newly enqueued job row, when enqueued.",
                    required: false,
                },
                FieldSchema {
                    name: "date_iso",
                    ty: TypeSchema::String,
                    comment: "The date the digest will cover, echoed back \
                        as `YYYY-MM-DD`.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: NAMESPACE,
            function: "unknown",
            description: "Unknown memory_tree controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_ingest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<tree_rpc::IngestRequest>(Value::Object(params))?;
        to_json(tree_rpc::ingest_rpc(&config, req).await?)
    })
}

fn handle_get_chunk(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<tree_rpc::GetChunkRequest>(Value::Object(params))?;
        to_json(tree_rpc::get_chunk_rpc(&config, req).await?)
    })
}

fn handle_trigger_digest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<tree_rpc::TriggerDigestRequest>(Value::Object(params))?;
        to_json(tree_rpc::trigger_digest_rpc(&config, req).await?)
    })
}

// ── New read RPCs (Memory-tab UI) ────────────────────────────────────────

fn handle_list_chunks(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let filter = parse_value::<read_rpc::ChunkFilter>(Value::Object(params))?;
        to_json(read_rpc::list_chunks_rpc(&config, filter).await?)
    })
}

fn handle_list_sources(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize, Default)]
        struct Req {
            #[serde(default)]
            user_email_hint: Option<String>,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params)).unwrap_or_default();
        to_json(read_rpc::list_sources_rpc(&config, req.user_email_hint).await?)
    })
}

fn handle_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            query: String,
            k: u32,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::search_rpc(&config, req.query, req.k).await?)
    })
}

fn handle_recall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            query: String,
            k: u32,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::recall_rpc(&config, req.query, req.k).await?)
    })
}

fn handle_entity_index_for(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            chunk_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::entity_index_for_rpc(&config, req.chunk_id).await?)
    })
}

fn handle_chunks_for_entity(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            entity_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::chunks_for_entity_rpc(&config, req.entity_id).await?)
    })
}

fn handle_top_entities(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            #[serde(default)]
            kind: Option<String>,
            limit: u32,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::top_entities_rpc(&config, req.kind, req.limit).await?)
    })
}

fn handle_chunk_score(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            chunk_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::chunk_score_rpc(&config, req.chunk_id).await?)
    })
}

fn handle_delete_chunk(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            chunk_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::delete_chunk_rpc(&config, req.chunk_id).await?)
    })
}

fn handle_get_llm(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(read_rpc::get_llm_rpc(&config).await?)
    })
}

fn handle_set_llm(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mut config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<read_rpc::SetLlmRequest>(Value::Object(params))?;
        to_json(read_rpc::set_llm_rpc(&mut config, req).await?)
    })
}

fn handle_graph_export(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize, Default)]
        struct Req {
            #[serde(default)]
            mode: Option<read_rpc::GraphMode>,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params)).unwrap_or_default();
        to_json(read_rpc::graph_export_rpc(&config, req.mode.unwrap_or_default()).await?)
    })
}

fn handle_flush_now(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(read_rpc::flush_now_rpc(&config).await?)
    })
}

fn handle_wipe_all(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(read_rpc::wipe_all_rpc(&config).await?)
    })
}

fn handle_reset_tree(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(read_rpc::reset_tree_rpc(&config).await?)
    })
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
