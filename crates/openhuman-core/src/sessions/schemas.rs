//! Controller schemas and JSON-RPC dispatchers for the durable run ledger.

use crate::core::runtime::DispatchCtx;
use crate::util::params::{
    json_output, new_correlation_id, optional_string, required_string, value_to_json,
};
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use tinyagents_session::run_ledger::{AgentRunListRequest, RunEventListRequest};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("run_ledger_list"),
        schema_for("run_ledger_get"),
        schema_for("run_ledger_events"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("run_ledger_list"),
            handler: handle_run_ledger_list,
        },
        RegisteredController {
            schema: schema_for("run_ledger_get"),
            handler: handle_run_ledger_get,
        },
        RegisteredController {
            schema: schema_for("run_ledger_events"),
            handler: handle_run_ledger_events,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "run_ledger_list" => ControllerSchema {
            namespace: "run_ledger",
            function: "list",
            description:
                "List durable agent/workflow run ledger rows with optional filters and pagination.",
            inputs: vec![
                optional_string("status", "Filter by run status."),
                optional_string("kind", "Filter by run kind."),
                optional_string("parentRunId", "Filter by parent run id."),
                optional_string("parentThreadId", "Filter by parent thread id."),
                optional_u64("limit", "Max runs to return (default 50, max 500)."),
                optional_u64("offset", "Pagination offset."),
            ],
            outputs: vec![json_output(
                "result",
                "AgentRunListResponse with runs array and count.",
            )],
        },
        "run_ledger_get" => ControllerSchema {
            namespace: "run_ledger",
            function: "get",
            description: "Get a durable agent/workflow run ledger row by id.",
            inputs: vec![required_string("id", "Run id.")],
            outputs: vec![json_output("run", "AgentRun payload or null.")],
        },
        "run_ledger_events" => ControllerSchema {
            namespace: "run_ledger",
            function: "events",
            description: "List recent durable events for a run, ordered by sequence.",
            inputs: vec![
                required_string("runId", "Run id."),
                optional_u64("afterSequence", "Only return events after this sequence."),
                optional_u64("limit", "Max events to return (default 100, max 1000)."),
            ],
            outputs: vec![json_output(
                "events",
                "RunEventListResponse with ordered events and count.",
            )],
        },
        _ => ControllerSchema {
            namespace: "run_ledger",
            function: "unknown",
            description: "Unknown run_ledger controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_run_ledger_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] list.entry");
        let config = ctx.load_config().await.inspect_err(|err| {
            log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] list.config_failed err={err}");
        })?;
        let request: AgentRunListRequest = if params.is_empty() {
            AgentRunListRequest::default()
        } else {
            serde_json::from_value(Value::Object(params)).map_err(|e| {
                let s = format!("invalid run ledger list params: {e}");
                log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] list.bad_params err={s}");
                s
            })?
        };
        let response = crate::storage::sessions::with_ledger_async(
            &config.workspace_dir,
            |ledger| async move { ledger.list_agent_runs(&request).await },
        )
        .await
        .map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] list.error err={s}");
            s
        })?;
        value_to_json(response)
    })
}

fn handle_run_ledger_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] get.entry");
        let config = ctx.load_config().await.inspect_err(|err| {
            log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] get.config_failed err={err}");
        })?;
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: id".to_string())?;
        let run = crate::storage::sessions::with_ledger_async(&config.workspace_dir, |ledger| async move {
            ledger.get_agent_run(id).await
        })
        .await
        .map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] get.error id={id} err={s}");
            s
        })?;
        value_to_json(serde_json::json!({ "run": run }))
    })
}

fn handle_run_ledger_events(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] events.entry");
        let config = ctx.load_config().await.inspect_err(|err| {
            log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] events.config_failed err={err}");
        })?;
        let request: RunEventListRequest =
            serde_json::from_value(Value::Object(params)).map_err(|e| {
                let s = format!("invalid run ledger events params: {e}");
                log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] events.bad_params err={s}");
                s
            })?;
        let response = crate::storage::sessions::with_ledger_async(
            &config.workspace_dir,
            |ledger| async move { ledger.list_recent_run_events(&request).await },
        )
        .await
        .map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "run_ledger_rpc", "[run_ledger_rpc][{cid}] events.error err={s}");
            s
        })?;
        value_to_json(response)
    })
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
