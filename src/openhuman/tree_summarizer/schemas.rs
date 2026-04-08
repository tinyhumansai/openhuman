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
