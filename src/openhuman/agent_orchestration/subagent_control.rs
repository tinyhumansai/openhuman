//! Controller schema + JSON-RPC dispatcher for user-driven control of detached
//! background sub-agents (`spawn_async_subagent`).
//!
//! Exposes `openhuman.subagent_cancel`: the frontend "Cancel" affordance in the
//! background-tasks drawer calls this to abort a still-running detached
//! sub-agent. Cancellation aborts the in-flight task via the
//! [`super::running_subagents`] registry and records a "cancelled" pseudo-
//! completion so the existing idle-gated delivery path
//! ([`super::background_delivery`]) surfaces it back in the parent chat.
//!
//! This is the *manual* counterpart to the *automatic* thread-close
//! cancellation in [`crate::openhuman::threads`]: there the thread is being
//! deleted (so nothing is delivered and the thread is tombstoned), whereas here
//! the thread stays alive and the user expects to see that their sub-agent was
//! cancelled.

use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent_orchestration::{background_completions, running_subagents};
use crate::rpc::RpcOutcome;

/// Controller schemas exposed for detached sub-agent control.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schema_for("subagent_cancel")]
}

/// Registered controllers (schema + handler) for detached sub-agent control.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schema_for("subagent_cancel"),
        handler: handle_subagent_cancel,
    }]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "subagent_cancel" => ControllerSchema {
            namespace: "subagent",
            function: "cancel",
            description: "Cancel a still-running detached background sub-agent by its spawn task \
                          id. Aborts the in-flight run and posts a 'cancelled' notice back into \
                          the parent chat thread. No-op (cancelled=false) if the sub-agent already \
                          finished or the id is unknown.",
            inputs: vec![
                required_str(
                    "taskId",
                    "Spawn task id (`sub-…`) of the background sub-agent.",
                ),
                optional_str(
                    "reason",
                    "Optional reason, included in the cancelled notice shown in chat.",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "{ cancelled: bool, taskId: string } — cancelled=false if nothing was running.",
            )],
        },
        _ => ControllerSchema {
            namespace: "subagent",
            function: "unknown",
            description: "unknown subagent control function",
            inputs: vec![],
            outputs: vec![],
        },
    }
}

fn handle_subagent_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        let task_id = require_str(&params, "taskId")?;
        let reason = opt_str(&params, "reason");
        log::debug!(
            target: "subagent_control_rpc",
            "[subagent_control_rpc][{cid}] cancel.entry task_id={task_id}"
        );

        let cancelled = match running_subagents::cancel_by_task(&task_id) {
            Some(meta) => {
                let summary = match reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                    Some(r) => format!(
                        "You cancelled this background sub-agent before it finished. Reason: {r}"
                    ),
                    None => {
                        "You cancelled this background sub-agent before it finished.".to_string()
                    }
                };
                // The thread is still alive (unlike the delete path), so we
                // record a completion that flows through the same idle-gated
                // delivery and surfaces the cancellation in chat.
                background_completions::record_completion(
                    meta.parent_session,
                    &task_id,
                    meta.agent_id,
                    summary,
                    meta.parent_thread_id,
                );
                true
            }
            None => false,
        };

        log::debug!(
            target: "subagent_control_rpc",
            "[subagent_control_rpc][{cid}] cancel.done task_id={task_id} cancelled={cancelled}"
        );
        to_json(json!({ "cancelled": cancelled, "taskId": task_id }))
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

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

/// Extract a required non-empty string param, **trimmed**, or an RPC-facing
/// error. Trimming matters for `taskId`: a whitespace-padded id would otherwise
/// pass validation yet never match the registry key in `cancel_by_task`.
fn require_str(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_controllers_match_schemas() {
        let schemas = all_controller_schemas();
        let registered = all_registered_controllers();
        assert_eq!(schemas.len(), registered.len());
        assert_eq!(schemas.len(), 1);
        assert_eq!(schema_for("subagent_cancel").namespace, "subagent");
        assert_eq!(schema_for("subagent_cancel").function, "cancel");
    }

    #[test]
    fn require_str_rejects_blank_and_missing() {
        let mut params = Map::new();
        assert!(require_str(&params, "taskId").is_err());
        params.insert("taskId".into(), json!("   "));
        assert!(require_str(&params, "taskId").is_err());
        params.insert("taskId".into(), json!("sub-1"));
        assert_eq!(require_str(&params, "taskId").unwrap(), "sub-1");
        // Whitespace-padded ids are trimmed so they match the registry key.
        params.insert("taskId".into(), json!("  sub-1  "));
        assert_eq!(require_str(&params, "taskId").unwrap(), "sub-1");
    }

    #[tokio::test]
    async fn cancel_unknown_task_is_a_noop_false() {
        let _lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut params = Map::new();
        params.insert("taskId".into(), json!("sub-does-not-exist"));
        let out = handle_subagent_cancel(params).await.expect("handler ok");
        // RpcOutcome wraps the payload under `data`.
        let cancelled = out
            .get("data")
            .and_then(|d| d.get("cancelled"))
            .or_else(|| out.get("cancelled"))
            .and_then(Value::as_bool);
        assert_eq!(cancelled, Some(false));
    }
}
