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
use crate::openhuman::agent::harness::run_queue::QueueMode;
use crate::openhuman::agent_orchestration::running_subagents::SteerError;
use crate::openhuman::agent_orchestration::{
    background_completions, running_subagents, subagent_sessions,
};
use crate::rpc::RpcOutcome;

/// Controller schemas exposed for detached sub-agent control.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schema_for("subagent_cancel"), schema_for("subagent_steer")]
}

/// Registered controllers (schema + handler) for detached sub-agent control.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("subagent_cancel"),
            handler: handle_subagent_cancel,
        },
        RegisteredController {
            schema: schema_for("subagent_steer"),
            handler: handle_subagent_steer,
        },
    ]
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
        "subagent_steer" => ControllerSchema {
            namespace: "subagent",
            function: "steer",
            description: "Inject a message into a still-running detached background sub-agent by \
                          spawn task id. This trusted RPC control mirrors the steer_subagent agent \
                          tool and returns immediately after the message is queued.",
            inputs: vec![
                required_str(
                    "taskId",
                    "Spawn task id (`sub-…`) of the running background sub-agent.",
                ),
                required_str(
                    "message",
                    "Instruction or context to queue for the running sub-agent.",
                ),
                optional_str("mode", "Optional queue mode: steer (default) or collect."),
            ],
            outputs: vec![json_output(
                "result",
                "{ steered: bool, taskId: string, mode: string }.",
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
                    meta.parent_session.clone(),
                    &task_id,
                    meta.agent_id.clone(),
                    summary,
                    meta.parent_thread_id.clone(),
                );
                if let Some(subagent_session_id) = meta.subagent_session_id {
                    let store = subagent_sessions::SubagentSessionStore::new(meta.workspace_dir);
                    if let Err(err) = subagent_sessions::mark_failed(
                        &store,
                        &subagent_session_id,
                        &task_id,
                        "cancelled by user".to_string(),
                    ) {
                        log::warn!(
                            target: "subagent_control_rpc",
                            "[subagent_control_rpc][{cid}] cancel.mark_failed_failed task_id={task_id} subagent_session_id={subagent_session_id} error={err}"
                        );
                    }
                }
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

fn handle_subagent_steer(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        let task_id = require_str(&params, "taskId")?;
        let message = require_str(&params, "message")?;
        let mode = match opt_str(&params, "mode").as_deref() {
            Some("collect") => QueueMode::Collect,
            _ => QueueMode::Steer,
        };
        log::debug!(
            target: "subagent_control_rpc",
            "[subagent_control_rpc][{cid}] steer.entry task_id={task_id} mode={mode} chars={}",
            message.chars().count()
        );

        match running_subagents::steer_control(&task_id, message, mode).await {
            Ok(()) => {
                log::debug!(
                    target: "subagent_control_rpc",
                    "[subagent_control_rpc][{cid}] steer.done task_id={task_id} mode={mode} steered=true"
                );
                to_json(json!({ "steered": true, "taskId": task_id, "mode": mode.to_string() }))
            }
            Err(err) => {
                let reason = match err {
                    SteerError::Unknown => "unknown",
                    SteerError::AlreadyDone => "already_done",
                    // `steer_control` is trusted UI/RPC control and currently
                    // does not perform parent ownership checks. Keep the arm
                    // for future trust-model changes that may return NotOwned.
                    SteerError::NotOwned => "not_owned",
                };
                log::debug!(
                    target: "subagent_control_rpc",
                    "[subagent_control_rpc][{cid}] steer.done task_id={task_id} mode={mode} steered=false reason={reason}"
                );
                to_json(json!({
                    "steered": false,
                    "taskId": task_id,
                    "mode": mode.to_string(),
                    "reason": reason,
                }))
            }
        }
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
        assert_eq!(schemas.len(), 2);
        assert_eq!(schema_for("subagent_cancel").namespace, "subagent");
        assert_eq!(schema_for("subagent_cancel").function, "cancel");
        assert_eq!(schema_for("subagent_steer").namespace, "subagent");
        assert_eq!(schema_for("subagent_steer").function, "steer");
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

    #[tokio::test]
    async fn steer_unknown_task_is_a_noop_false() {
        let _lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut params = Map::new();
        params.insert("taskId".into(), json!("sub-does-not-exist"));
        params.insert("message".into(), json!("redirect"));
        let out = handle_subagent_steer(params).await.expect("handler ok");
        let data = out.get("data").unwrap_or(&out);
        assert_eq!(data.get("steered").and_then(Value::as_bool), Some(false));
        assert_eq!(data.get("reason").and_then(Value::as_str), Some("unknown"));
    }
}
