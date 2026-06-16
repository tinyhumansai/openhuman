//! Controller schemas and handlers for `openhuman.skill_runtime_*`.
//!
//! This namespace is the CLI/RPC-friendly execution surface for installed
//! skills. The older `workflows_*` run/log/cancel controllers are kept for
//! compatibility, but new scripts should call `skill_runtime_*` directly.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::workflows::run_log;
use crate::openhuman::workflows::schemas::resolve_workspace_dir;
use crate::rpc::RpcOutcome;

use super::ops::{resolve_runtimes, RuntimeRequirement};
use super::spawn_workflow_run_background;

#[derive(Debug, Deserialize)]
struct RunParams {
    skill_id: String,
    #[serde(default)]
    inputs: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct CancelParams {
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct RecentRunsParams {
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ReadRunLogParams {
    run_id: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct ResolveRuntimesParams {
    #[serde(default)]
    runtime: Option<String>,
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

pub fn all_skill_runtime_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        skill_runtime_schemas("run"),
        skill_runtime_schemas("cancel"),
        skill_runtime_schemas("recent_runs"),
        skill_runtime_schemas("read_run_log"),
        skill_runtime_schemas("resolve_runtimes"),
        skill_runtime_schemas("schemas"),
    ]
}

pub fn all_skill_runtime_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: skill_runtime_schemas("run"),
            handler: handle_run,
        },
        RegisteredController {
            schema: skill_runtime_schemas("cancel"),
            handler: handle_cancel,
        },
        RegisteredController {
            schema: skill_runtime_schemas("recent_runs"),
            handler: handle_recent_runs,
        },
        RegisteredController {
            schema: skill_runtime_schemas("read_run_log"),
            handler: handle_read_run_log,
        },
        RegisteredController {
            schema: skill_runtime_schemas("resolve_runtimes"),
            handler: handle_resolve_runtimes,
        },
        RegisteredController {
            schema: skill_runtime_schemas("schemas"),
            handler: handle_schemas,
        },
    ]
}

pub fn skill_runtime_schemas(function: &str) -> ControllerSchema {
    match function {
        "run" => ControllerSchema {
            namespace: "skill_runtime",
            function: "run",
            description: "Start an installed skill in the background and return a run id plus log path.",
            inputs: vec![
                FieldSchema {
                    name: "skill_id",
                    ty: TypeSchema::String,
                    comment: "Installed skill id (directory name / workflow id).",
                    required: true,
                },
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Json,
                    comment: "Optional JSON object of input values declared by the skill.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::String,
                    comment: "Background run id.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::String,
                    comment: "Always `started`.",
                    required: true,
                },
                FieldSchema {
                    name: "skill_id",
                    ty: TypeSchema::String,
                    comment: "Resolved installed skill id.",
                    required: true,
                },
                FieldSchema {
                    name: "log",
                    ty: TypeSchema::String,
                    comment: "Run log path.",
                    required: true,
                },
            ],
        },
        "cancel" => ControllerSchema {
            namespace: "skill_runtime",
            function: "cancel",
            description: "Request cancellation of an in-flight skill run.",
            inputs: vec![FieldSchema {
                name: "run_id",
                ty: TypeSchema::String,
                comment: "Run id returned by skill_runtime_run.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::String,
                    comment: "Echoed run id.",
                    required: true,
                },
                FieldSchema {
                    name: "cancelled",
                    ty: TypeSchema::Bool,
                    comment: "Whether a live run was found and signalled.",
                    required: true,
                },
            ],
        },
        "recent_runs" => ControllerSchema {
            namespace: "skill_runtime",
            function: "recent_runs",
            description: "List recent skill runs from the workspace run-log directory.",
            inputs: vec![
                FieldSchema {
                    name: "skill_id",
                    ty: TypeSchema::String,
                    comment: "Optional skill id filter.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Maximum runs to return, capped at 100.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Json,
                comment: "Recent run summaries.",
                required: true,
            }],
        },
        "read_run_log" => ControllerSchema {
            namespace: "skill_runtime",
            function: "read_run_log",
            description: "Read a slice of a skill run log by run id.",
            inputs: vec![
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::String,
                    comment: "Run id returned by skill_runtime_run.",
                    required: true,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::U64,
                    comment: "Byte offset to start reading from.",
                    required: false,
                },
                FieldSchema {
                    name: "max_bytes",
                    ty: TypeSchema::U64,
                    comment: "Maximum bytes to return, capped at 256 KiB.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "content",
                ty: TypeSchema::Json,
                comment: "Run-log slice.",
                required: true,
            }],
        },
        "resolve_runtimes" => ControllerSchema {
            namespace: "skill_runtime",
            function: "resolve_runtimes",
            description: "Resolve the reusable Node/Python runtimes used by skill execution and return binary paths for production smoke scripts.",
            inputs: vec![FieldSchema {
                name: "runtime",
                ty: TypeSchema::String,
                comment: "Runtime to resolve: all (default), node, or python.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "runtimes",
                ty: TypeSchema::Json,
                comment: "Resolved runtime status objects with enabled, available, source, version, binary, bin_dir, and error.",
                required: true,
            }],
        },
        "schemas" => ControllerSchema {
            namespace: "skill_runtime",
            function: "schemas",
            description: "Return the skill_runtime controller schemas for CLI/RPC smoke-test script generation.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "schemas",
                ty: TypeSchema::Json,
                comment: "Array of skill_runtime controller schemas.",
                required: true,
            }],
        },
        other => panic!("unknown skill_runtime schema function: {other}"),
    }
}

fn handle_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<RunParams>(params)?;
        tracing::info!(skill_id = %payload.skill_id, "[skill_runtime][rpc] run");
        let started = spawn_workflow_run_background(payload.skill_id, payload.inputs).await?;
        to_json(RpcOutcome::new(
            serde_json::json!({
                "run_id": started.run_id,
                "status": "started",
                "skill_id": started.workflow_id,
                "log": started.log_path.display().to_string(),
            }),
            Vec::new(),
        ))
    })
}

fn handle_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<CancelParams>(params)?;
        let cancelled = run_log::cancel_run(&payload.run_id);
        tracing::info!(run_id = %payload.run_id, cancelled, "[skill_runtime][rpc] cancel");
        to_json(RpcOutcome::new(
            serde_json::json!({ "run_id": payload.run_id, "cancelled": cancelled }),
            Vec::new(),
        ))
    })
}

fn handle_recent_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<RecentRunsParams>(params)?;
        let limit = payload.limit.unwrap_or(20).min(100) as usize;
        let workspace = resolve_workspace_dir().await;
        let runs = run_log::scan_runs(&workspace, payload.skill_id.as_deref(), limit);
        tracing::debug!(
            count = runs.len(),
            filter = ?payload.skill_id,
            limit,
            "[skill_runtime][rpc] recent_runs"
        );
        to_json(RpcOutcome::new(
            serde_json::json!({ "runs": runs }),
            Vec::new(),
        ))
    })
}

fn handle_read_run_log(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<ReadRunLogParams>(params)?;
        let workspace = resolve_workspace_dir().await;
        let path = run_log::find_run_log_path(&workspace, &payload.run_id).ok_or_else(|| {
            format!(
                "skill_runtime_read_run_log: unknown run_id '{}'",
                payload.run_id
            )
        })?;
        let offset = payload.offset.unwrap_or(0);
        let max_bytes = payload.max_bytes.unwrap_or(64 * 1024).min(256 * 1024) as usize;
        let slice = run_log::read_run_log_slice(&path, offset, max_bytes)
            .map_err(|e| format!("skill_runtime_read_run_log: read failed: {e}"))?;
        to_json(RpcOutcome::new(slice, Vec::new()))
    })
}

fn handle_resolve_runtimes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<ResolveRuntimesParams>(params)?;
        let requirement = RuntimeRequirement::from_optional(payload.runtime.as_deref())?;
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|error| format!("skill_runtime_resolve_runtimes: load config: {error:#}"))?;
        let outcome = resolve_runtimes(&config, requirement).await;
        to_json(RpcOutcome::new(outcome, Vec::new()))
    })
}

fn handle_schemas(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        to_json(RpcOutcome::new(
            serde_json::json!({ "schemas": all_skill_runtime_controller_schemas() }),
            Vec::new(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_cover_runtime_cli_surface() {
        let functions: Vec<_> = all_skill_runtime_controller_schemas()
            .into_iter()
            .map(|schema| schema.function)
            .collect();
        assert_eq!(
            functions,
            vec![
                "run",
                "cancel",
                "recent_runs",
                "read_run_log",
                "resolve_runtimes",
                "schemas"
            ]
        );
        assert!(all_skill_runtime_registered_controllers()
            .iter()
            .all(|controller| controller.schema.namespace == "skill_runtime"));
    }

    #[test]
    fn run_schema_uses_skill_id_not_workflow_id() {
        let schema = skill_runtime_schemas("run");
        assert_eq!(schema.namespace, "skill_runtime");
        assert!(schema.inputs.iter().any(|field| field.name == "skill_id"));
    }

    #[test]
    fn resolve_runtimes_schema_is_cli_friendly() {
        let schema = skill_runtime_schemas("resolve_runtimes");
        assert_eq!(schema.namespace, "skill_runtime");
        assert_eq!(schema.function, "resolve_runtimes");
        assert!(schema.inputs.iter().any(|field| field.name == "runtime"));
    }
}
