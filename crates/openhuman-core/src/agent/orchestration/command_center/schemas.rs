//! Controller schema + JSON-RPC dispatcher for the background agent command
//! center. Exposes `openhuman.agent_work_list` (read-only) over the durable
//! run ledger. Handlers delegate to [`super::ops`]; no business logic here.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use tinyagents_session::run_ledger::command_center::ControlVerb;

use crate::agent::harness::host_definition::AgentDefinitionRegistry;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

/// Controller schemas exposed by the command center.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("agent_work_list"),
        schema_for("agent_work_control"),
    ]
}

/// Registered controllers (schema + handler) for the command center.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("agent_work_list"),
            handler: handle_agent_work_list,
        },
        RegisteredController {
            schema: schema_for("agent_work_control"),
            handler: handle_agent_work_control,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "agent_work_list" => ControllerSchema {
            namespace: "agent_work",
            function: "list",
            description: "List recent background agent runs grouped by command-center status \
                          bucket (needs_input / working / completed / failed / stopped).",
            inputs: vec![optional_u64(
                "limit",
                "Max recent runs to scan (default 200, max 500).",
            )],
            outputs: vec![json_output(
                "result",
                "CommandCenterView with five status groups and a total count.",
            )],
        },
        "agent_work_control" => ControllerSchema {
            namespace: "agent_work",
            function: "control",
            description: "Apply a control verb to one background agent run, transitioning its \
                          durable ledger status and recording a run event. Actions: stop \
                          (cancel a live run), retry (re-queue a failed/stopped run), continue \
                          (answer an awaiting-input run), follow_up (record a new instruction). \
                          `message` is required for continue and follow_up.",
            inputs: vec![
                required_str("runId", "Ledger run id from a command-center row."),
                required_str(
                    "action",
                    "Control verb: stop | retry | continue | follow_up.",
                ),
                optional_str(
                    "message",
                    "User message — required for continue and follow_up.",
                ),
                optional_str("reason", "Optional reason recorded when stopping."),
            ],
            outputs: vec![json_output(
                "row",
                "The updated AgentWorkRow after the transition.",
            )],
        },
        _ => ControllerSchema {
            namespace: "agent_work",
            function: "unknown",
            description: "unknown command center function",
            inputs: vec![],
            outputs: vec![],
        },
    }
}

fn handle_agent_work_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "command_center_rpc", "[command_center_rpc][{cid}] list.entry");
        let config = ctx.load_config()
            .await
            .inspect_err(|err| {
                log::warn!(target: "command_center_rpc", "[command_center_rpc][{cid}] list.config_failed err={err}");
            })?;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let view = crate::agent::session_store::with_ledger_async(&config.workspace_dir, |ledger| async move {
            ledger.list_agent_work(limit, &resolve_display_name).await
        })
        .await
        .map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "command_center_rpc", "[command_center_rpc][{cid}] list.error err={s}");
            s
        })?;
        to_json(view)
    })
}

fn handle_agent_work_control(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "command_center_rpc", "[command_center_rpc][{cid}] control.entry");
        let config = ctx.load_config()
            .await
            .inspect_err(|err| {
                log::warn!(target: "command_center_rpc", "[command_center_rpc][{cid}] control.config_failed err={err}");
            })?;
        let run_id = require_str(&params, "runId")?;
        let action = require_str(&params, "action")?;
        let verb = ControlVerb::parse(&action).ok_or_else(|| {
            let s = format!("unknown control action: {action}");
            log::warn!(target: "command_center_rpc", "[command_center_rpc][{cid}] control.unknown_action action={action}");
            s
        })?;
        let message = opt_str(&params, "message");
        let reason = opt_str(&params, "reason");
        log::debug!(target: "command_center_rpc", "[command_center_rpc][{cid}] control.parsed run_id={run_id} verb={}", verb.as_str());
        let row = crate::agent::session_store::with_ledger_async(&config.workspace_dir, |ledger| {
            let run_id = run_id.clone();
            let message = message.clone();
            let reason = reason.clone();
            async move {
                ledger
                    .apply_control(
                        &run_id,
                        verb,
                        message.as_deref(),
                        reason.as_deref(),
                        &resolve_display_name,
                    )
                    .await
            }
        })
        .await
        .map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "command_center_rpc", "[command_center_rpc][{cid}] control.error err={s}");
            s
        })?;
        log::debug!(target: "command_center_rpc", "[command_center_rpc][{cid}] control.success run_id={run_id} status={}", row.status);
        to_json(serde_json::json!({ "row": row }))
    })
}

/// Resolve an agent id to its registry display name, if the registry is up and
/// the agent is known. Returns `None` otherwise (e.g. custom/removed agents).
fn resolve_display_name(agent_id: &str) -> Option<String> {
    AgentDefinitionRegistry::current().and_then(|registry| {
        registry
            .get(agent_id)
            .map(|definition| definition.display_name().to_string())
    })
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    Outcome::new(value, vec![]).into_cli_compatible_json()
}

fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
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

/// Extract a required non-empty string param, or an RPC-facing error.
fn require_str(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing required param: {key}"))
}

/// Extract an optional non-empty string param.
fn opt_str(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
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
#[path = "schemas_tests.rs"]
mod tests;
