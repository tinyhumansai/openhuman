//! Controller schemas + handler adapters for the life-capture domain.
//!
//! Controllers exposed:
//!   - `life_capture.get_stats` — index size, per-source counts, last-ingest ts
//!   - `life_capture.search`    — hybrid (vector + keyword + recency) search
//!
//! Handlers translate the raw `Map<String, Value>` params into typed calls into
//! the domain functions in `rpc.rs`. Runtime state (the `PersonalIndex` + the
//! `Embedder`) is fetched from the process-global runtime in `runtime.rs`,
//! which F14 wires at startup.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::life_capture::{rpc, runtime};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("get_stats"), schemas("search")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_stats"),
            handler: handle_get_stats,
        },
        RegisteredController {
            schema: schemas("search"),
            handler: handle_search,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get_stats" => ControllerSchema {
            namespace: "life_capture",
            function: "get_stats",
            description: "Index size, per-source item counts, and most recent ingest timestamp.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "total_items",
                    ty: TypeSchema::I64,
                    comment: "Total number of items in the personal index.",
                    required: true,
                },
                FieldSchema {
                    name: "by_source",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Object {
                        fields: vec![
                            FieldSchema {
                                name: "source",
                                ty: TypeSchema::String,
                                comment: "Source identifier (e.g. 'gmail', 'calendar').",
                                required: true,
                            },
                            FieldSchema {
                                name: "count",
                                ty: TypeSchema::I64,
                                comment: "Number of items from this source.",
                                required: true,
                            },
                        ],
                    })),
                    comment: "Per-source item counts ordered by source name.",
                    required: true,
                },
                FieldSchema {
                    name: "last_ingest_ts",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Unix-seconds timestamp of the most recent item, or null when empty.",
                    required: true,
                },
            ],
        },
        "search" => ControllerSchema {
            namespace: "life_capture",
            function: "search",
            description: "Hybrid search over the personal index (vector + keyword + recency).",
            inputs: vec![
                FieldSchema {
                    name: "text",
                    ty: TypeSchema::String,
                    comment: "Query text. Required and non-empty.",
                    required: true,
                },
                FieldSchema {
                    name: "k",
                    ty: TypeSchema::U64,
                    comment: "Maximum number of hits to return. Defaults to 10, capped at 100.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "hits",
                ty: TypeSchema::Array(Box::new(TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "item_id",
                            ty: TypeSchema::String,
                            comment: "UUID of the matching item.",
                            required: true,
                        },
                        FieldSchema {
                            name: "score",
                            ty: TypeSchema::F64,
                            comment: "Hybrid relevance score (higher is better).",
                            required: true,
                        },
                        FieldSchema {
                            name: "snippet",
                            ty: TypeSchema::String,
                            comment: "Short surrounding text for citation rendering.",
                            required: true,
                        },
                        FieldSchema {
                            name: "source",
                            ty: TypeSchema::String,
                            comment: "Source identifier (e.g. 'gmail').",
                            required: true,
                        },
                        FieldSchema {
                            name: "subject",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "Item subject, when available.",
                            required: false,
                        },
                        FieldSchema {
                            name: "ts",
                            ty: TypeSchema::I64,
                            comment: "Unix-seconds timestamp of the item.",
                            required: true,
                        },
                    ],
                })),
                comment: "Ranked hits, best first.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "life_capture",
            function: "unknown",
            description: "Unknown life_capture function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Error message.",
                required: true,
            }],
        },
    }
}

fn handle_get_stats(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let idx = runtime::get_index().map_err(|e| e.to_string())?;
        to_json(rpc::handle_get_stats(&idx).await?)
    })
}

fn handle_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let idx = runtime::get_index().map_err(|e| e.to_string())?;
        let embedder = runtime::get_embedder().map_err(|e| e.to_string())?;
        let text = read_required_string(&params, "text")?;
        let k = read_optional_u64(&params, "k")?.unwrap_or(10) as usize;
        to_json(rpc::handle_search(&idx, &embedder, text, k).await?)
    })
}

fn read_required_string(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    match params.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
            type_name(other)
        )),
        None => Err(format!("missing required param '{key}'")),
    }
}

fn read_optional_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected unsigned integer")),
        Some(other) => Err(format!(
            "invalid '{key}': expected unsigned integer, got {}",
            type_name(other)
        )),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_get_stats_has_no_inputs_and_three_outputs() {
        let s = schemas("get_stats");
        assert_eq!(s.namespace, "life_capture");
        assert_eq!(s.function, "get_stats");
        assert!(s.inputs.is_empty());
        let names: Vec<_> = s.outputs.iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["total_items", "by_source", "last_ingest_ts"]);
    }

    #[test]
    fn schemas_search_requires_text_and_optional_k() {
        let s = schemas("search");
        let text = s.inputs.iter().find(|f| f.name == "text").unwrap();
        assert!(text.required);
        let k = s.inputs.iter().find(|f| f.name == "k").unwrap();
        assert!(!k.required);
    }

    #[test]
    fn schemas_unknown_returns_placeholder() {
        let s = schemas("does-not-exist");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn all_controller_schemas_lists_both_functions() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(names, vec!["get_stats", "search"]);
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        let regs = all_registered_controllers();
        assert_eq!(regs.len(), 2);
        let names: Vec<_> = regs.iter().map(|c| c.schema.function).collect();
        assert_eq!(names, vec!["get_stats", "search"]);
    }
}
