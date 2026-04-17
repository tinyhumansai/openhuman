//! Controller schemas and RPC handler wiring for `tree_summarizer`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("ingest"),
        schemas("run"),
        schemas("query"),
        schemas("status"),
        schemas("rebuild"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("ingest"),
            handler: handle_ingest,
        },
        RegisteredController {
            schema: schemas("run"),
            handler: handle_run,
        },
        RegisteredController {
            schema: schemas("query"),
            handler: handle_query,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("rebuild"),
            handler: handle_rebuild,
        },
    ]
}

fn namespace_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "namespace",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "ingest" => ControllerSchema {
            namespace: "tree_summarizer",
            function: "ingest",
            description: "Append raw content to the tree summarizer ingestion buffer.",
            inputs: vec![
                namespace_input("Namespace (scope) for the summary tree."),
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Raw content to buffer for summarization.",
                    required: true,
                },
                FieldSchema {
                    name: "timestamp",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional RFC3339 timestamp; defaults to now.",
                    required: false,
                },
                FieldSchema {
                    name: "metadata",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional metadata JSON.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Confirmation of buffered content.",
                required: true,
            }],
        },
        "run" => ControllerSchema {
            namespace: "tree_summarizer",
            function: "run",
            description:
                "Trigger the summarization job: drain buffer, create hour leaf, propagate upward.",
            inputs: vec![namespace_input(
                "Namespace to run the summarization job for.",
            )],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Hour leaf node or skip status.",
                required: true,
            }],
        },
        "query" => ControllerSchema {
            namespace: "tree_summarizer",
            function: "query",
            description: "Read a tree node and its direct children.",
            inputs: vec![
                namespace_input("Namespace of the summary tree."),
                FieldSchema {
                    name: "node_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Node ID to query; defaults to 'root'.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "The node and its children.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "tree_summarizer",
            function: "status",
            description: "Get tree metadata: node count, depth, date range.",
            inputs: vec![namespace_input("Namespace of the summary tree.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Tree status metadata.",
                required: true,
            }],
        },
        "rebuild" => ControllerSchema {
            namespace: "tree_summarizer",
            function: "rebuild",
            description:
                "Rebuild the entire summary tree from hour leaves upward (re-summarizes all levels).",
            inputs: vec![namespace_input("Namespace to rebuild.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Tree status after rebuild.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "tree_summarizer",
            function: "unknown",
            description: "Unknown tree_summarizer controller function.",
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

// ── Handlers ───────────────────────────────────────────────────────────

fn handle_ingest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let namespace = read_required::<String>(&params, "namespace")?;
        let content = read_required::<String>(&params, "content")?;
        let timestamp = read_optional_timestamp(&params, "timestamp")?;
        let metadata = read_optional::<Value>(&params, "metadata")?;
        to_json(
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_ingest(
                &config,
                &namespace,
                &content,
                timestamp,
                metadata.as_ref(),
            )
            .await?,
        )
    })
}

fn handle_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let namespace = read_required::<String>(&params, "namespace")?;
        to_json(
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_run(&config, &namespace)
                .await?,
        )
    })
}

fn handle_query(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let namespace = read_required::<String>(&params, "namespace")?;
        let node_id = read_optional::<String>(&params, "node_id")?;
        to_json(
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_query(
                &config,
                &namespace,
                node_id.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let namespace = read_required::<String>(&params, "namespace")?;
        to_json(
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_status(&config, &namespace)
                .await?,
        )
    })
}

fn handle_rebuild(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let namespace = read_required::<String>(&params, "namespace")?;
        to_json(
            crate::openhuman::tree_summarizer::rpc::tree_summarizer_rebuild(&config, &namespace)
                .await?,
        )
    })
}

// ── Param helpers ──────────────────────────────────────────────────────

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn read_optional_timestamp(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| Some(dt.with_timezone(&chrono::Utc)))
            .map_err(|e| format!("invalid '{key}': {e}")),
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
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
    use serde_json::json;

    #[test]
    fn all_schemas_returns_five() {
        assert_eq!(all_controller_schemas().len(), 5);
    }

    #[test]
    fn all_controllers_returns_five() {
        assert_eq!(all_registered_controllers().len(), 5);
    }

    #[test]
    fn all_use_tree_summarizer_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "tree_summarizer");
            assert!(!s.description.is_empty());
        }
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, ctrl.schema.function);
        }
    }

    #[test]
    fn known_functions_resolve() {
        for fn_name in ["ingest", "run", "query", "status", "rebuild"] {
            let s = schemas(fn_name);
            assert_ne!(s.function, "unknown", "{fn_name} fell through");
        }
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = schemas("nonexistent");
        assert_eq!(s.function, "unknown");
    }

    #[test]
    fn ingest_requires_namespace_and_content() {
        let s = schemas("ingest");
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert!(required.contains(&"namespace"));
        assert!(required.contains(&"content"));
    }

    #[test]
    fn query_requires_namespace() {
        let s = schemas("query");
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert!(required.contains(&"namespace"));
    }

    #[test]
    fn status_requires_namespace() {
        let s = schemas("status");
        assert!(s.inputs.iter().any(|f| f.name == "namespace" && f.required));
    }

    // ── Param helper tests ──────────────────────────────────────────

    #[test]
    fn read_required_parses_string() {
        let mut m = Map::new();
        m.insert("key".into(), Value::String("val".into()));
        let result: String = read_required(&m, "key").unwrap();
        assert_eq!(result, "val");
    }

    #[test]
    fn read_required_errors_on_missing() {
        let m = Map::new();
        let err = read_required::<String>(&m, "key").unwrap_err();
        assert!(err.contains("missing required"));
    }

    #[test]
    fn read_optional_returns_none_for_missing() {
        let m = Map::new();
        let result: Option<String> = read_optional(&m, "key").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_optional_returns_none_for_null() {
        let mut m = Map::new();
        m.insert("key".into(), Value::Null);
        let result: Option<String> = read_optional(&m, "key").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_optional_returns_some_for_value() {
        let mut m = Map::new();
        m.insert("key".into(), Value::String("val".into()));
        let result: Option<String> = read_optional(&m, "key").unwrap();
        assert_eq!(result, Some("val".into()));
    }

    #[test]
    fn read_optional_timestamp_valid_rfc3339() {
        let mut m = Map::new();
        m.insert("ts".into(), Value::String("2026-04-17T12:00:00Z".into()));
        let result = read_optional_timestamp(&m, "ts").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn read_optional_timestamp_invalid_format() {
        let mut m = Map::new();
        m.insert("ts".into(), Value::String("not-a-date".into()));
        assert!(read_optional_timestamp(&m, "ts").is_err());
    }

    #[test]
    fn read_optional_timestamp_non_string() {
        let mut m = Map::new();
        m.insert("ts".into(), json!(12345));
        assert!(read_optional_timestamp(&m, "ts").is_err());
    }

    #[test]
    fn read_optional_timestamp_none_for_missing() {
        let m = Map::new();
        assert!(read_optional_timestamp(&m, "ts").unwrap().is_none());
    }

    // ── type_name ───────────────────────────────────────────────────

    #[test]
    fn type_name_covers_all_variants() {
        assert_eq!(type_name(&Value::Null), "null");
        assert_eq!(type_name(&Value::Bool(true)), "bool");
        assert_eq!(type_name(&json!(42)), "number");
        assert_eq!(type_name(&json!("s")), "string");
        assert_eq!(type_name(&json!([1])), "array");
        assert_eq!(type_name(&json!({})), "object");
    }

    // ── namespace_input helper ───────────────────────────────────────

    #[test]
    fn namespace_input_is_required_string() {
        let f = namespace_input("test");
        assert_eq!(f.name, "namespace");
        assert!(f.required);
        assert!(matches!(f.ty, TypeSchema::String));
    }
}
