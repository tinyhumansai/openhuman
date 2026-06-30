use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_artifacts"),
        schemas("get_artifact"),
        schemas("delete_artifact"),
        schemas("regenerate"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_artifacts"),
            handler: handle_list_artifacts,
        },
        RegisteredController {
            schema: schemas("get_artifact"),
            handler: handle_get_artifact,
        },
        RegisteredController {
            schema: schemas("delete_artifact"),
            handler: handle_delete_artifact,
        },
        RegisteredController {
            schema: schemas("regenerate"),
            handler: handle_regenerate,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_artifacts" => ControllerSchema {
            namespace: "ai",
            function: "list_artifacts",
            description: "List agent-generated artifacts in the workspace with pagination.",
            inputs: vec![
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Zero-based index of the first artifact to return; defaults to 0.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment:
                        "Maximum number of artifacts to return; defaults to 50, capped at 200.",
                    required: false,
                },
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "When set, return only artifacts whose meta.json was written for this \
                         chat thread (#3226). Lets ChatFilesPanel repopulate from disk on a \
                         fresh redux slice / new device. Absent (or null) returns all artifacts.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "artifacts",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ArtifactMeta"))),
                    comment: "Artifact metadata records sorted by created_at descending.",
                    required: true,
                },
                FieldSchema {
                    name: "total",
                    ty: TypeSchema::U64,
                    comment: "Total number of artifacts in the workspace before pagination.",
                    required: true,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::U64,
                    comment: "Offset used for this page.",
                    required: true,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Limit used for this page.",
                    required: true,
                },
            ],
        },
        "get_artifact" => ControllerSchema {
            namespace: "ai",
            function: "get_artifact",
            description: "Retrieve metadata for a single artifact by ID.",
            inputs: vec![artifact_id_input("Identifier of the artifact to retrieve.")],
            outputs: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Unique artifact identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "kind",
                    ty: TypeSchema::String,
                    comment: "Category of the artifact (presentation, document, image, other).",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "Human-readable title.",
                    required: true,
                },
                FieldSchema {
                    name: "path",
                    ty: TypeSchema::String,
                    comment: "Relative path from the artifacts root.",
                    required: true,
                },
                FieldSchema {
                    name: "size_bytes",
                    ty: TypeSchema::U64,
                    comment: "Artifact file size in bytes.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::String,
                    comment: "Lifecycle status (pending, ready, failed).",
                    required: true,
                },
                FieldSchema {
                    name: "created_at",
                    ty: TypeSchema::String,
                    comment: "UTC timestamp when the artifact was created (ISO 8601).",
                    required: true,
                },
                FieldSchema {
                    name: "absolute_path",
                    ty: TypeSchema::String,
                    comment: "Absolute on-disk path to the artifact directory.",
                    required: true,
                },
            ],
        },
        "delete_artifact" => ControllerSchema {
            namespace: "ai",
            function: "delete_artifact",
            description: "Delete an artifact and all its associated files from the workspace.",
            inputs: vec![artifact_id_input("Identifier of the artifact to delete.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "artifact_id",
                            ty: TypeSchema::String,
                            comment: "Identifier that was requested for deletion.",
                            required: true,
                        },
                        FieldSchema {
                            name: "deleted",
                            ty: TypeSchema::Bool,
                            comment: "True when the artifact was successfully deleted.",
                            required: true,
                        },
                    ],
                },
                comment: "Deletion result payload.",
                required: true,
            }],
        },
        "regenerate" => ControllerSchema {
            namespace: "ai",
            function: "regenerate",
            description: "Re-run the producing tool for an existing artifact using its persisted creation args, reusing the same artifact id so the chat card swaps in place. Drives the failed-card Retry affordance (#3162).",
            inputs: vec![
                artifact_id_input("Identifier of the artifact to regenerate."),
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Chat thread to route the regenerated artifact's pending/ready/failed events to.",
                    required: true,
                },
                FieldSchema {
                    name: "client_id",
                    ty: TypeSchema::String,
                    comment: "Socket client id (web channel) to address the regenerated artifact's events to.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "artifact_id",
                            ty: TypeSchema::String,
                            comment: "Identifier that was regenerated (unchanged — reused in place).",
                            required: true,
                        },
                        FieldSchema {
                            name: "regenerated",
                            ty: TypeSchema::Bool,
                            comment: "True when the producing tool was re-dispatched.",
                            required: true,
                        },
                        FieldSchema {
                            name: "is_error",
                            ty: TypeSchema::Bool,
                            comment: "True when the re-dispatched generation itself reported an error (the card is flipped to failed via socket regardless).",
                            required: true,
                        },
                    ],
                },
                comment: "Regeneration ack payload.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "ai",
            function: "unknown",
            description: "Unknown artifacts controller function.",
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

fn artifact_id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "artifact_id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn handle_list_artifacts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let offset = read_optional_u64(&params, "offset")?
            .map(|raw| {
                usize::try_from(raw).map_err(|_| "offset is too large for usize".to_string())
            })
            .transpose()?;
        let limit = read_optional_u64(&params, "limit")?
            .map(|raw| usize::try_from(raw).map_err(|_| "limit is too large for usize".to_string()))
            .transpose()?;
        let thread_id = read_optional_string(&params, "thread_id")?;
        to_json(
            crate::openhuman::artifacts::ops::ai_list_artifacts(
                &config,
                offset,
                limit,
                thread_id.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_get_artifact(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        to_json(
            crate::openhuman::artifacts::ops::ai_get_artifact(&config, artifact_id.trim()).await?,
        )
    })
}

fn handle_delete_artifact(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        to_json(
            crate::openhuman::artifacts::ops::ai_delete_artifact(&config, artifact_id.trim())
                .await?,
        )
    })
}

fn handle_regenerate(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        let thread_id = read_required::<String>(&params, "thread_id")?;
        let client_id = read_required::<String>(&params, "client_id")?;
        to_json(
            crate::openhuman::artifacts::ops::ai_regenerate(
                &config,
                artifact_id.trim(),
                thread_id.trim(),
                client_id.trim(),
            )
            .await?,
        )
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional_string(params: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            // Treat whitespace-only input as absent so the handler doesn't
            // try to match an artifact against an unfilterable empty string.
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
            type_name(other)
        )),
    }
}

fn read_optional_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
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
    use serde_json::json;

    // ── schemas() branch coverage ───────────────────────────────────

    #[test]
    fn schemas_list_artifacts_has_pagination_inputs_and_correct_outputs() {
        let s = schemas("list_artifacts");
        assert_eq!(s.namespace, "ai");
        assert_eq!(s.function, "list_artifacts");
        let input_names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
        assert!(input_names.contains(&"offset"));
        assert!(input_names.contains(&"limit"));
        assert!(s.inputs.iter().all(|f| !f.required));
        let output_names: Vec<_> = s.outputs.iter().map(|f| f.name).collect();
        assert!(output_names.contains(&"artifacts"));
        assert!(output_names.contains(&"total"));
        assert!(output_names.contains(&"offset"));
        assert!(output_names.contains(&"limit"));
    }

    #[test]
    fn schemas_get_artifact_requires_artifact_id() {
        let s = schemas("get_artifact");
        assert_eq!(s.namespace, "ai");
        assert_eq!(s.function, "get_artifact");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "artifact_id");
        assert!(s.inputs[0].required);
        // Output should be flat fields matching the actual JSON response shape
        let output_names: Vec<_> = s.outputs.iter().map(|f| f.name).collect();
        assert!(output_names.contains(&"id"));
        assert!(output_names.contains(&"kind"));
        assert!(output_names.contains(&"title"));
        assert!(output_names.contains(&"path"));
        assert!(output_names.contains(&"size_bytes"));
        assert!(output_names.contains(&"status"));
        assert!(output_names.contains(&"created_at"));
        assert!(output_names.contains(&"absolute_path"));
        // Must NOT have an opaque "artifact" wrapper
        assert!(!output_names.contains(&"artifact"));
    }

    #[test]
    fn schemas_delete_artifact_has_artifact_id_input_and_result_output() {
        let s = schemas("delete_artifact");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "artifact_id");
        assert!(s.inputs[0].required);
        assert_eq!(s.outputs[0].name, "result");
        if let TypeSchema::Object { fields } = &s.outputs[0].ty {
            let names: Vec<_> = fields.iter().map(|f| f.name).collect();
            assert!(names.contains(&"artifact_id"));
            assert!(names.contains(&"deleted"));
        } else {
            panic!("expected object output type");
        }
    }

    #[test]
    fn schemas_unknown_function_returns_placeholder_with_error_output() {
        let s = schemas("does-not-exist");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    // ── registry helpers ────────────────────────────────────────────

    #[test]
    fn all_controller_schemas_covers_every_supported_function() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(
            names,
            vec![
                "list_artifacts",
                "get_artifact",
                "delete_artifact",
                "regenerate"
            ]
        );
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        let controllers = all_registered_controllers();
        assert_eq!(controllers.len(), 4);
        let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
        assert_eq!(
            names,
            vec![
                "list_artifacts",
                "get_artifact",
                "delete_artifact",
                "regenerate"
            ]
        );
    }

    #[test]
    fn schemas_regenerate_requires_artifact_id_thread_and_client() {
        let s = schemas("regenerate");
        assert_eq!(s.function, "regenerate");
        let input_names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
        assert_eq!(input_names, vec!["artifact_id", "thread_id", "client_id"]);
        assert!(s.inputs.iter().all(|f| f.required));
        if let TypeSchema::Object { fields } = &s.outputs[0].ty {
            let names: Vec<_> = fields.iter().map(|f| f.name).collect();
            assert!(names.contains(&"artifact_id"));
            assert!(names.contains(&"regenerated"));
            assert!(names.contains(&"is_error"));
        } else {
            panic!("expected object output type");
        }
    }

    // ── read_required ───────────────────────────────────────────────

    #[test]
    fn read_required_returns_value_for_present_key() {
        let mut params = Map::new();
        params.insert("artifact_id".into(), json!("abc"));
        let got: String = read_required(&params, "artifact_id").unwrap();
        assert_eq!(got, "abc");
    }

    #[test]
    fn read_required_errors_when_key_missing() {
        let params = Map::new();
        let err = read_required::<String>(&params, "artifact_id").unwrap_err();
        assert!(err.contains("missing required param 'artifact_id'"));
    }

    // ── read_optional_u64 ───────────────────────────────────────────

    #[test]
    fn read_optional_u64_absent_key_is_none() {
        assert_eq!(read_optional_u64(&Map::new(), "limit").unwrap(), None);
    }

    #[test]
    fn read_optional_u64_explicit_null_is_none() {
        let mut params = Map::new();
        params.insert("limit".into(), Value::Null);
        assert_eq!(read_optional_u64(&params, "limit").unwrap(), None);
    }

    #[test]
    fn read_optional_u64_accepts_unsigned_integer() {
        let mut params = Map::new();
        params.insert("limit".into(), json!(50));
        assert_eq!(read_optional_u64(&params, "limit").unwrap(), Some(50));
    }

    #[test]
    fn read_optional_u64_rejects_negative_number() {
        let mut params = Map::new();
        params.insert("limit".into(), json!(-1));
        let err = read_optional_u64(&params, "limit").unwrap_err();
        assert!(err.contains("expected unsigned integer"));
    }

    // ── type_name ───────────────────────────────────────────────────

    #[test]
    fn type_name_reports_each_json_variant() {
        assert_eq!(type_name(&Value::Null), "null");
        assert_eq!(type_name(&json!(true)), "bool");
        assert_eq!(type_name(&json!(1)), "number");
        assert_eq!(type_name(&json!("s")), "string");
        assert_eq!(type_name(&json!([])), "array");
        assert_eq!(type_name(&json!({})), "object");
    }
}
