//! Controller schemas for Phase 4 retrieval tools (#710).
//!
//! Registered JSON-RPC methods:
//! - `openhuman.memory_tree_query_source`
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
use crate::openhuman::memory_tree::retrieval::rpc as retrieval_rpc;
use crate::rpc::RpcOutcome;

const NAMESPACE: &str = "memory_tree";

/// Return one [`ControllerSchema`] per Phase 4 retrieval tool. Used by
/// the controller registry to publish the `memory_tree.*` schemas.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("query_source"),
        schemas("cover_window"),
        schemas("search_entities"),
        schemas("drill_down"),
        schemas("fetch_leaves"),
    ]
}

/// Return one [`RegisteredController`] per Phase 4 retrieval tool — schema
/// paired with its dispatch handler. Wired into `core::all` at startup.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("query_source"),
            handler: handle_query_source,
        },
        RegisteredController {
            schema: schemas("cover_window"),
            handler: handle_cover_window,
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

/// Look up the [`ControllerSchema`] for a single retrieval `function`
/// name. Unknown names return a placeholder schema with an `error` field.
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
        "cover_window" => ControllerSchema {
            namespace: NAMESPACE,
            function: "cover_window",
            description: "Return the MINIMUM set of nodes covering all memory in a time \
                 window `[since_ms, until_ms]` (epoch-millis). Emits the coarsest summary \
                 whose whole subtree falls inside the window, and raw leaf chunks for \
                 anything not covered by such a summary (boundary content and not-yet-\
                 summarised chunks). Optional `source_id` / `source_kind` scope the result. \
                 Hits are grouped by source and ordered ascending by start time. Use this \
                 for time-bounded recaps (e.g. a last-24h morning brief) instead of \
                 `query_source`, which returns all-time summaries.",
            inputs: vec![
                FieldSchema {
                    name: "since_ms",
                    ty: TypeSchema::I64,
                    comment: "Inclusive window start, epoch-milliseconds.",
                    required: true,
                },
                FieldSchema {
                    name: "until_ms",
                    ty: TypeSchema::I64,
                    comment: "Inclusive window end, epoch-milliseconds.",
                    required: true,
                },
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
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max hits (default 200).",
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

fn handle_cover_window(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<retrieval_rpc::CoverWindowRequest>(Value::Object(params))?;
        to_json(retrieval_rpc::cover_window_rpc(&config, req).await?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_cover_every_registered_retrieval_function() {
        let schemas = all_controller_schemas();
        let functions: Vec<&str> = schemas.iter().map(|s| s.function).collect();
        assert_eq!(
            functions,
            vec![
                "query_source",
                "cover_window",
                "search_entities",
                "drill_down",
                "fetch_leaves",
            ]
        );
    }

    #[test]
    fn registered_controllers_use_memory_tree_namespace() {
        let controllers = all_registered_controllers();
        assert_eq!(controllers.len(), 5);
        assert!(controllers.iter().all(|c| c.schema.namespace == NAMESPACE));
    }

    #[test]
    fn unknown_schema_returns_error_output() {
        let schema = schemas("not_a_real_function");
        assert_eq!(schema.namespace, NAMESPACE);
        assert_eq!(schema.function, "unknown");
        assert_eq!(schema.outputs.len(), 1);
        assert_eq!(schema.outputs[0].name, "error");
    }
}
