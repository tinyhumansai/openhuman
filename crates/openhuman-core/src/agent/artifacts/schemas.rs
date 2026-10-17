use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_artifacts"),
        schemas("get_artifact"),
        schemas("delete_artifact"),
        schemas("regenerate"),
        schemas("save_artifact_version"),
        schemas("share_artifact"),
        schemas("read_artifact_body"),
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
        RegisteredController {
            schema: schemas("save_artifact_version"),
            handler: handle_save_artifact_version,
        },
        RegisteredController {
            schema: schemas("share_artifact"),
            handler: handle_share_artifact,
        },
        RegisteredController {
            schema: schemas("read_artifact_body"),
            handler: handle_read_artifact_body,
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
                    comment: "File name relative to its root: the files folder, or the artifacts root for a legacy record.",
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
        "save_artifact_version" => ControllerSchema {
            namespace: "ai",
            function: "save_artifact_version",
            description: "Save the user's edit of a versioned artifact as a new operator-authored version; updates its file and card.",
            inputs: vec![
                artifact_id_input("Identifier of the versioned artifact."),
                FieldSchema {
                    name: "body",
                    ty: TypeSchema::String,
                    comment: "Full content of the new version.",
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
                            comment: "Identifier of the artifact.",
                            required: true,
                        },
                        FieldSchema {
                            name: "version",
                            ty: TypeSchema::U64,
                            comment: "The newest version number after the call.",
                            required: true,
                        },
                        FieldSchema {
                            name: "saved",
                            ty: TypeSchema::Bool,
                            comment: "False when the body matched the newest version and nothing was appended.",
                            required: true,
                        },
                    ],
                },
                comment: "Save result payload.",
                required: true,
            }],
        },
        "read_artifact_body" => ControllerSchema {
            namespace: "ai",
            function: "read_artifact_body",
            description: "The newest version of an artifact for the preview panel: its text, a document converted to Markdown, or the file's path.",
            inputs: vec![artifact_id_input("Identifier of the versioned artifact.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "kind",
                            ty: TypeSchema::String,
                            comment: "text, markdown, html, image or file.",
                            required: true,
                        },
                        FieldSchema {
                            name: "title",
                            ty: TypeSchema::String,
                            comment: "The artifact's title.",
                            required: true,
                        },
                        FieldSchema {
                            name: "version",
                            ty: TypeSchema::U64,
                            comment: "The newest version number.",
                            required: true,
                        },
                        FieldSchema {
                            name: "body",
                            ty: TypeSchema::String,
                            comment: "The content, or the file's path when is_text is false.",
                            required: true,
                        },
                        FieldSchema {
                            name: "is_text",
                            ty: TypeSchema::Bool,
                            comment: "Whether body is content rather than a path.",
                            required: true,
                        },
                        FieldSchema {
                            name: "converted",
                            ty: TypeSchema::Bool,
                            comment: "Whether body is Markdown converted from a document file.",
                            required: true,
                        },
                    ],
                },
                comment: "Preview payload.",
                required: true,
            }],
        },
        "share_artifact" => ControllerSchema {
            namespace: "ai",
            function: "share_artifact",
            description: "Share a version of an artifact through cloud file storage and return its link (reused while it still works).",
            inputs: vec![
                artifact_id_input("Identifier of the versioned artifact."),
                FieldSchema {
                    name: "version",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Version to share; the newest when absent.",
                    required: false,
                },
                FieldSchema {
                    name: "public",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "A stable public link instead of a private, expiring one.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "url",
                            ty: TypeSchema::String,
                            comment: "The shareable link.",
                            required: true,
                        },
                        FieldSchema {
                            name: "version",
                            ty: TypeSchema::U64,
                            comment: "The version shared.",
                            required: true,
                        },
                        FieldSchema {
                            name: "public",
                            ty: TypeSchema::Bool,
                            comment: "Whether the link is public.",
                            required: true,
                        },
                        FieldSchema {
                            name: "expires_at_millis",
                            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                            comment: "When the link stops working, if it expires.",
                            required: false,
                        },
                        FieldSchema {
                            name: "reused",
                            ty: TypeSchema::Bool,
                            comment: "True when an existing live link was returned without uploading.",
                            required: true,
                        },
                    ],
                },
                comment: "Share result payload.",
                required: true,
            }],
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

fn handle_list_artifacts(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
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
            crate::agent::artifacts::ops::ai_list_artifacts(
                &config,
                offset,
                limit,
                thread_id.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_get_artifact(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        to_json(crate::agent::artifacts::ops::ai_get_artifact(&config, artifact_id.trim()).await?)
    })
}

fn handle_save_artifact_version(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        let body = read_required::<String>(&params, "body")?;
        to_json(
            crate::agent::artifacts::ops::ai_save_artifact_version(
                &config,
                artifact_id.trim(),
                &body,
            )
            .await?,
        )
    })
}

fn handle_read_artifact_body(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        let body = crate::agent::artifacts::preview::read_body(&config, artifact_id.trim()).await?;
        to_json(Outcome::new(body, vec![]))
    })
}

fn handle_share_artifact(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        let version = read_optional_u64(&params, "version")?
            .map(|v| u32::try_from(v).map_err(|_| "invalid 'version': too large".to_string()))
            .transpose()?;
        let public = match params.get("public") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(other) => {
                return Err(format!(
                    "invalid 'public': expected bool, got {}",
                    type_name(other)
                ))
            }
        };
        to_json(
            crate::agent::artifacts::ops::ai_share_artifact(
                &config,
                artifact_id.trim(),
                version,
                public,
            )
            .await?,
        )
    })
}

fn handle_delete_artifact(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        to_json(
            crate::agent::artifacts::ops::ai_delete_artifact(&config, artifact_id.trim()).await?,
        )
    })
}

fn handle_regenerate(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let artifact_id = read_required::<String>(&params, "artifact_id")?;
        let thread_id = read_required::<String>(&params, "thread_id")?;
        let client_id = read_required::<String>(&params, "client_id")?;
        to_json(
            crate::agent::artifacts::ops::ai_regenerate(
                &ctx,
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

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
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
#[path = "schemas_tests.rs"]
mod tests;
