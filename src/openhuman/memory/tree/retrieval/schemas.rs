//! Controller schemas for Phase 4 retrieval tools (#710).
//!
//! Registered JSON-RPC methods:
//! - `openhuman.memory_tree_query_source`
//! - `openhuman.memory_tree_query_global`
//! - `openhuman.memory_tree_query_topic`
//! - `openhuman.memory_tree_search_entities`
//! - `openhuman.memory_tree_drill_down`
//! - `openhuman.memory_tree_fetch_leaves`
//!
//! Handlers delegate to [`super::rpc`]. Namespaces reuse `memory_tree` to
//! keep the tool surface tightly grouped with the Phase 1-3 ingest
//! controllers.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::retrieval::rpc as retrieval_rpc;
use crate::rpc::RpcOutcome;

const NAMESPACE: &str = "memory_tree";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("query_source"),
        schemas("query_global"),
        schemas("query_topic"),
        schemas("search_entities"),
        schemas("drill_down"),
        schemas("fetch_leaves"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("query_source"),
            handler: handle_query_source,
        },
        RegisteredController {
            schema: schemas("query_global"),
            handler: handle_query_global,
        },
        RegisteredController {
            schema: schemas("query_topic"),
            handler: handle_query_topic,
        },
        RegisteredController {
            schema: schemas("search_entities"),
            handler: handle_search_entities,
        },
        RegisteredController {
            schema: schemas("drill_down"),
            handler: handle_drill_down,
        },
        RegisteredController {
            schema: schemas("fetch_leaves"),
            handler: handle_fetch_leaves,
        },
    ]
}

/// Flat output shape for all `query_*` tools. Mirrors `QueryResponse`'s
/// serde layout (three top-level fields) so schema-driven callers see the
/// same structure the handler actually emits. Flagged on PR #831 CodeRabbit
/// review — previously declared as a single `response: QueryResponse` field.
fn query_response_outputs() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "hits",
            ty: TypeSchema::Array(Box::new(TypeSchema::Ref("RetrievalHit"))),
            comment: "Ordered list of hits (summaries and/or leaves).",
            required: true,
        },
        FieldSchema {
            name: "total",
            ty: TypeSchema::U64,
            comment: "Candidate count before truncation by `limit`.",
            required: true,
        },
        FieldSchema {
            name: "truncated",
            ty: TypeSchema::Bool,
            comment: "True when `total > hits.len()`.",
            required: true,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "query_source" => ControllerSchema {
            namespace: NAMESPACE,
            function: "query_source",
            description: "Return summaries from one or more per-source trees. \
                 Filter by `source_id` (exact), `source_kind` (chat/email/document), \
                 and/or `time_window_days`. Results are newest-first and capped at `limit`. \
                 Pass `query` to rerank candidates by cosine similarity against the \
                 stored embedding (legacy rows without an embedding fall to the bottom).",
            inputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Exact source id (e.g. `slack:#eng`, `gmail:abc`).",
                    required: false,
                },
                FieldSchema {
                    name: "source_kind",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["chat", "email", "document"],
                    })),
                    comment: "Source kind filter when no exact id is known.",
                    required: false,
                },
                FieldSchema {
                    name: "time_window_days",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Only return summaries whose time range overlaps the \
                     last N days.",
                    required: false,
                },
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional natural-language query — when present, \
                     candidates are reranked by cosine similarity to the query's \
                     embedding. Candidates without stored embeddings sort last.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max hits (default 10).",
                    required: false,
                },
            ],
            outputs: query_response_outputs(),
        },
        "query_global" => ControllerSchema {
            namespace: NAMESPACE,
            function: "query_global",
            description: "Return the global digest for the last N days. Wraps \
                 `global_tree::recap`; the returned hit carries `child_ids` pointing \
                 at the folded per-day summary ids for drill-down.",
            inputs: vec![FieldSchema {
                name: "window_days",
                ty: TypeSchema::U64,
                comment: "Lookback window in days (e.g. 7 for weekly recap).",
                required: true,
            }],
            outputs: query_response_outputs(),
        },
        "query_topic" => ControllerSchema {
            namespace: NAMESPACE,
            function: "query_topic",
            description: "Return summaries / chunks associated with a canonical \
                 entity id across every tree (source, topic, global). Also returns \
                 the topic tree's root if one has materialised for the entity. \
                 Sorted by (score DESC, timestamp DESC), or by cosine similarity \
                 if `query` is provided.",
            inputs: vec![
                FieldSchema {
                    name: "entity_id",
                    ty: TypeSchema::String,
                    comment: "Canonical id (e.g. `email:alice@example.com`, `topic:phoenix`).",
                    required: true,
                },
                FieldSchema {
                    name: "time_window_days",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Only return hits whose time range overlaps the last N days.",
                    required: false,
                },
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional natural-language query — when present, \
                     candidates are reranked by cosine similarity to the query's \
                     embedding. Candidates without stored embeddings sort last.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max hits (default 10).",
                    required: false,
                },
            ],
            outputs: query_response_outputs(),
        },
        "search_entities" => ControllerSchema {
            namespace: NAMESPACE,
            function: "search_entities",
            description: "Free-text LIKE search over the entity index. Matches \
                 against canonical ids and surface forms. Aggregated by canonical \
                 id — `mention_count` reflects total occurrences.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Substring to match (case-insensitive).",
                    required: true,
                },
                FieldSchema {
                    name: "kinds",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::Enum {
                            variants: vec![
                                "email",
                                "url",
                                "handle",
                                "hashtag",
                                "person",
                                "organization",
                                "location",
                                "event",
                                "product",
                                "misc",
                                "topic",
                            ],
                        },
                    )))),
                    comment: "Optional EntityKind filter — restrict to these kinds only.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max matches (default 5, clamped to 100).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "matches",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("EntityMatch"))),
                comment: "Aggregated matches, strongest count first.",
                required: true,
            }],
        },
        "drill_down" => ControllerSchema {
            namespace: NAMESPACE,
            function: "drill_down",
            description: "Walk a summary node's children one step (or more if \
                 `max_depth > 1`). Returns leaf chunks when the input is an L1 \
                 summary, or lower-level summaries when the input is L2+. \
                 When `query` is provided, children are reranked by cosine \
                 similarity to the query embedding — useful when a summary \
                 has many children and only the relevant ones are needed.",
            inputs: vec![
                FieldSchema {
                    name: "node_id",
                    ty: TypeSchema::String,
                    comment: "Id of the summary (or leaf) to expand.",
                    required: true,
                },
                FieldSchema {
                    name: "max_depth",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "How many levels down to walk (default 1).",
                    required: false,
                },
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional free-text query; when set, children are \
                        reranked by cosine similarity to the query embedding \
                        and unembedded children sort to the bottom.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Optional cap on returned hits, applied after rerank.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "hits",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("RetrievalHit"))),
                comment: "Hydrated child hits; empty on leaves or unknown ids.",
                required: true,
            }],
        },
        "fetch_leaves" => ControllerSchema {
            namespace: NAMESPACE,
            function: "fetch_leaves",
            description: "Batch-fetch raw chunk rows by id. Max 20 per call — the \
                 excess is silently truncated. Missing ids are skipped.",
            inputs: vec![FieldSchema {
                name: "chunk_ids",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Chunk ids to hydrate. Capped at 20 per call.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "hits",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("RetrievalHit"))),
                comment: "Hydrated leaf hits in input order (missing ids skipped).",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: NAMESPACE,
            function: "unknown",
            description: "Unknown memory_tree retrieval controller function.",
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

// ── Handlers ────────────────────────────────────────────────────────────

fn handle_query_source(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<retrieval_rpc::QuerySourceRequest>(Value::Object(params))?;
        to_json(retrieval_rpc::query_source_rpc(&config, req).await?)
    })
}

fn handle_query_global(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<retrieval_rpc::QueryGlobalRequest>(Value::Object(params))?;
        to_json(retrieval_rpc::query_global_rpc(&config, req).await?)
    })
}

fn handle_query_topic(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<retrieval_rpc::QueryTopicRequest>(Value::Object(params))?;
        to_json(retrieval_rpc::query_topic_rpc(&config, req).await?)
    })
}

fn handle_search_entities(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<retrieval_rpc::SearchEntitiesRequest>(Value::Object(params))?;
        to_json(retrieval_rpc::search_entities_rpc(&config, req).await?)
    })
}

fn handle_drill_down(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<retrieval_rpc::DrillDownRequest>(Value::Object(params))?;
        to_json(retrieval_rpc::drill_down_rpc(&config, req).await?)
    })
}

fn handle_fetch_leaves(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<retrieval_rpc::FetchLeavesRequest>(Value::Object(params))?;
        to_json(retrieval_rpc::fetch_leaves_rpc(&config, req).await?)
    })
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
