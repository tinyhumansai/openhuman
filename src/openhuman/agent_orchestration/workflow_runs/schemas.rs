//! Controller schemas + JSON-RPC dispatchers for durable workflow runs.
//!
//! Read surface (PR1): list builtin definitions, list durable runs, get a run
//! by id. Execution surface (PR2): start / stop / resume, delegating to the
//! [`super::engine`] phase scheduler. Namespace `workflow_run` — distinct from
//! the existing `workflows` domain (SKILL.md/WORKFLOW.md discovery).

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::session_db::run_ledger::WorkflowRunListRequest;
use crate::rpc::RpcOutcome;

/// Controller schemas exposed by the workflow-runs module.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("workflow_run_list_definitions"),
        schema_for("workflow_run_list"),
        schema_for("workflow_run_get"),
        schema_for("workflow_run_start"),
        schema_for("workflow_run_stop"),
        schema_for("workflow_run_resume"),
    ]
}

/// Registered controllers (schema + handler) for workflow runs.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("workflow_run_list_definitions"),
            handler: handle_list_definitions,
        },
        RegisteredController {
            schema: schema_for("workflow_run_list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schema_for("workflow_run_get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schema_for("workflow_run_start"),
            handler: handle_start,
        },
        RegisteredController {
            schema: schema_for("workflow_run_stop"),
            handler: handle_stop,
        },
        RegisteredController {
            schema: schema_for("workflow_run_resume"),
            handler: handle_resume,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "workflow_run_list_definitions" => ControllerSchema {
            namespace: "workflow_run",
            function: "list_definitions",
            description: "List available declarative workflow definitions (builtins).",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "WorkflowDefinitionListResponse with definitions and count.",
            )],
        },
        "workflow_run_list" => ControllerSchema {
            namespace: "workflow_run",
            function: "list",
            description: "List durable workflow runs with optional filters and pagination.",
            inputs: vec![
                optional_str("definitionId", "Filter by workflow definition id."),
                optional_str("status", "Filter by run status."),
                optional_str("parentThreadId", "Filter by parent thread id."),
                optional_u64("limit", "Max runs to return (default 50, max 500)."),
                optional_u64("offset", "Pagination offset."),
            ],
            outputs: vec![json_output(
                "result",
                "WorkflowRunListResponse with runs array and count.",
            )],
        },
        "workflow_run_get" => ControllerSchema {
            namespace: "workflow_run",
            function: "get",
            description: "Get a durable workflow run by id.",
            inputs: vec![required_str("id", "Workflow run id.")],
            outputs: vec![json_output("workflowRun", "WorkflowRun payload or null.")],
        },
        "workflow_run_start" => ControllerSchema {
            namespace: "workflow_run",
            function: "start",
            description: "Start a workflow run from a definition id; returns the created \
                          Running run. Execution proceeds asynchronously — poll \
                          workflow_run_get for progress.",
            inputs: vec![
                required_str("definitionId", "Workflow definition id to run."),
                optional_json("input", "Run input (e.g. { \"question\": \"...\" })."),
                optional_str("parentThreadId", "Originating thread id, for lineage."),
            ],
            outputs: vec![json_output(
                "workflowRun",
                "The created WorkflowRun (status running).",
            )],
        },
        "workflow_run_stop" => ControllerSchema {
            namespace: "workflow_run",
            function: "stop",
            description: "Stop a running workflow after its current phase; marks the run \
                          interrupted and aborts in-flight child agents.",
            inputs: vec![required_str("id", "Workflow run id.")],
            outputs: vec![json_output(
                "workflowRun",
                "Updated WorkflowRun payload or null.",
            )],
        },
        "workflow_run_resume" => ControllerSchema {
            namespace: "workflow_run",
            function: "resume",
            description: "Resume an interrupted workflow run, skipping phases already \
                          completed and continuing from the first incomplete phase.",
            inputs: vec![required_str("id", "Workflow run id.")],
            outputs: vec![json_output(
                "workflowRun",
                "The resumed WorkflowRun (status running).",
            )],
        },
        other => unreachable!("unknown workflow_run schema: {other}"),
    }
}

fn handle_list_definitions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list_definitions.entry");
        let response = super::ops::list_definitions();
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list_definitions.success count={}", response.count);
        to_json(response)
    })
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list.config_failed err={err}");
        })?;
        let request: WorkflowRunListRequest = if params.is_empty() {
            log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list.branch=default_request");
            WorkflowRunListRequest::default()
        } else {
            log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list.branch=parsed_params");
            serde_json::from_value(Value::Object(params)).map_err(|e| {
                let s = format!("invalid workflow run list params: {e}");
                log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list.bad_params err={s}");
                s
            })?
        };
        let response = super::ops::list_runs(&config, &request).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list.error err={s}");
            s
        })?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] list.success count={}", response.count);
        to_json(response)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] get.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] get.config_failed err={err}");
        })?;
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: id".to_string())?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] get.id={id}");
        let run = super::ops::get_run(&config, id).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] get.error id={id} err={s}");
            s
        })?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] get.success id={id} found={}", run.is_some());
        to_json(serde_json::json!({ "workflowRun": run }))
    })
}

fn handle_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] start.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] start.config_failed err={err}");
        })?;
        let definition_id = params
            .get("definitionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: definitionId".to_string())?
            .to_string();
        let input = params.get("input").cloned().unwrap_or(Value::Null);
        let parent_thread_id = params
            .get("parentThreadId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] start.definition={definition_id}");
        let run = super::engine::start_workflow_run(
            &config,
            &definition_id,
            input,
            parent_thread_id,
        )
        .await
        .map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] start.error err={s}");
            s
        })?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] start.success id={}", run.id);
        to_json(serde_json::json!({ "workflowRun": run }))
    })
}

fn handle_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] stop.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] stop.config_failed err={err}");
        })?;
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: id".to_string())?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] stop.id={id}");
        let run = super::engine::stop_workflow_run(&config, id).await.map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] stop.error id={id} err={s}");
            s
        })?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] stop.success id={id} found={}", run.is_some());
        to_json(serde_json::json!({ "workflowRun": run }))
    })
}

fn handle_resume(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] resume.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] resume.config_failed err={err}");
        })?;
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: id".to_string())?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] resume.id={id}");
        let run = super::engine::resume_workflow_run(&config, id).await.map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] resume.error id={id} err={s}");
            s
        })?;
        log::debug!(target: "workflow_run_rpc", "[workflow_run_rpc][{cid}] resume.success id={}", run.id);
        to_json(serde_json::json!({ "workflowRun": run }))
    })
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    RpcOutcome::new(value, vec![]).into_cli_compatible_json()
}

fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn required_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

fn optional_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_controllers_match_schemas() {
        let schemas = all_controller_schemas();
        let registered = all_registered_controllers();
        assert_eq!(schemas.len(), registered.len());
        assert_eq!(
            schemas.len(),
            6,
            "expected 3 read + 3 execution controllers"
        );
        assert!(schemas.iter().all(|s| s.namespace == "workflow_run"));
        assert_eq!(schema_for("workflow_run_get").function, "get");
        assert_eq!(schema_for("workflow_run_start").function, "start");
        assert_eq!(schema_for("workflow_run_stop").function, "stop");
        assert_eq!(schema_for("workflow_run_resume").function, "resume");
    }
}
