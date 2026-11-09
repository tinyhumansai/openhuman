//! Controller schema + JSON-RPC dispatcher for user-driven control of detached
//! background sub-agents (`spawn_subagent`).
//!
//! Exposes `openhuman.subagent_cancel`: the frontend "Cancel" affordance in the
//! background-tasks drawer calls this to abort a still-running detached
//! sub-agent. Cancellation aborts the in-flight task via the
//! [`super::running_subagents`] registry and records a "cancelled" pseudo-
//! completion so the existing idle-gated delivery path
//! ([`super::background_delivery`]) surfaces it back in the parent chat.
//!
//! This is the *manual* counterpart to the *automatic* thread-close
//! cancellation in [`crate::threads`]: there the thread is being
//! deleted (so nothing is delivered and the thread is tombstoned), whereas here
//! the thread stays alive and the user expects to see that their sub-agent was
//! cancelled.

use crate::core::runtime::DispatchCtx;
use serde_json::{json, Map, Value};

use crate::agent::orchestration::running_subagents::CancelledSubagent;
use crate::agent::orchestration::{background_completions, running_subagents, subagent_sessions};
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

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
                "{ cancelled: bool, taskId: string, outcome?: \"completed\" | \"failed\" | \"unknown\" } \
                 — cancelled=false if nothing was running; outcome then says how the run had \
                 ended, or \"unknown\" if the core no longer knows the task.",
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

fn handle_subagent_cancel(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        let task_id = require_str(&params, "taskId")?;
        let reason = opt_str(&params, "reason");
        log::debug!(
            target: "subagent_control_rpc",
            "[subagent_control_rpc][{cid}] cancel.entry task_id={task_id}"
        );

        // `outcome` is set only when nothing was cancelled: how the run ended
        // if the registry still knew it, else "unknown" (never registered, or
        // already swept). The card settles on it instead of assuming success.
        let mut cancelled_subagent = running_subagents::cancel_by_task_with_workspace_reason(
            &ctx,
            &task_id,
            None,
            reason.as_deref(),
        )
        .await?;
        if cancelled_subagent.is_none()
            && ctx.resolved().is_none()
            && !crate::core::runtime::is_saas()
        {
            let config = crate::config::ops::load_config_in(&ctx)
                .await
                .map_err(|error| {
                    format!("subagent_cancel: failed to resolve active workspace: {error}")
                })?;
            cancelled_subagent = running_subagents::cancel_by_task_with_workspace_reason(
                &ctx,
                &task_id,
                Some(&config.workspace_dir),
                reason.as_deref(),
            )
            .await?;
        }
        let (cancelled, outcome) = match cancelled_subagent {
            // Finished before the click landed (it stays registered until the
            // terminal sweep). Its outcome is already recorded and delivered;
            // announcing a cancellation would overwrite a completed session
            // with "cancelled by user" and post a false notice into the chat.
            Some(CancelledSubagent {
                already_finished: Some(finished),
                agent_id,
                ..
            }) => {
                log::debug!(
                    target: "subagent_control_rpc",
                    "[subagent_control_rpc][{cid}] cancel.already_finished task_id={task_id} agent_id={agent_id} outcome={}",
                    finished.as_str()
                );
                (false, Some(finished.as_str()))
            }
            Some(meta) => {
                if meta.driver_managed {
                    // Active driver jobs finish through the awaited host hook.
                    // A paused job has already returned from that hook, so this
                    // handler settles and reports that one terminal transition.
                    if let Some(paused) = meta.driver_pause.as_ref() {
                        let transitioned = settle_paused_driver_cancel(
                            &ctx,
                            paused,
                            meta.parent_epoch,
                            reason.as_deref(),
                        )
                        .await?;
                        if !transitioned {
                            (false, Some("unknown"))
                        } else {
                            (true, None)
                        }
                    } else {
                        (true, None)
                    }
                } else {
                    let summary = match reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                        Some(r) => format!(
                            "You cancelled this background sub-agent before it finished. Reason: {r}"
                        ),
                        None => "You cancelled this background sub-agent before it finished."
                            .to_string(),
                    };
                    // The thread is still alive (unlike the delete path), so we
                    // record a completion that flows through the same idle-gated
                    // delivery and surfaces the cancellation in chat.
                    background_completions::record_outcome_with_parent_epoch(
                        &ctx,
                        &meta.workspace_dir,
                        &meta.parent_session,
                        &task_id,
                        meta.agent_id.clone(),
                        summary,
                        meta.parent_thread_id.clone(),
                        meta.parent_epoch,
                        background_completions::BackgroundAgentOutcome::Completed,
                    )
                    .await;
                    if let Some(subagent_session_id) = meta.subagent_session_id.as_deref() {
                        let store =
                            subagent_sessions::SubagentSessionStore::new(meta.workspace_dir);
                        if let Err(err) = subagent_sessions::mark_failed(
                            &store,
                            subagent_session_id,
                            &task_id,
                            "cancelled by user".to_string(),
                        ) {
                            log::warn!(
                                target: "subagent_control_rpc",
                                "[subagent_control_rpc][{cid}] cancel.mark_failed_failed task_id={task_id} subagent_session_id={subagent_session_id} error={err}"
                            );
                        }
                    }
                    (true, None)
                }
            }
            None => (false, Some("unknown")),
        };

        log::debug!(
            target: "subagent_control_rpc",
            "[subagent_control_rpc][{cid}] cancel.done task_id={task_id} cancelled={cancelled} outcome={outcome:?}"
        );
        let mut reply = json!({ "cancelled": cancelled, "taskId": task_id });
        if let Some(outcome) = outcome {
            reply["outcome"] = json!(outcome);
        }
        to_json(reply)
    })
}

async fn settle_paused_driver_cancel(
    ctx: &DispatchCtx,
    paused: &subagent_sessions::DriverTaskCancellation,
    parent_epoch: Option<u64>,
    reason: Option<&str>,
) -> Result<bool, String> {
    if paused.lifecycle_key.is_none() {
        return Err("subagent_cancel: paused driver child has no lifecycle key".into());
    }
    let store = subagent_sessions::open_for_context(
        ctx,
        paused.workspace_dir.clone(),
        paused.parent_session.clone(),
        paused.parent_thread_id.clone(),
        None,
    )
    .map_err(|error| format!("subagent_cancel: failed to open paused child store: {error}"))?;
    let error = match reason.map(str::trim).filter(|reason| !reason.is_empty()) {
        Some(reason) => format!("cancelled by user. Reason: {reason}"),
        None => "cancelled by user".to_string(),
    };
    let transitioned = store
        .mark_failed(&paused.subagent_session_id, &paused.task_id, error.clone())
        .await
        .map_err(|error| format!("subagent_cancel: failed to settle paused child: {error}"))?;
    if transitioned {
        crate::agent::orchestration::completion_owners::finish_driver_task(ctx, &paused.task_id);
        let parent_epoch = paused
            .parent_thread_id
            .as_ref()
            .map(|_| parent_epoch.unwrap_or(0));
        background_completions::record_outcome_with_parent_epoch(
            ctx,
            &paused.workspace_dir,
            &paused.parent_session,
            &paused.task_id,
            &paused.agent_id,
            format!(
                "[SUBAGENT_FAILED] the async sub-agent errored before producing a result: {error}"
            ),
            paused.parent_thread_id.clone(),
            parent_epoch,
            background_completions::BackgroundAgentOutcome::Failed,
        )
        .await;
    }
    Ok(transitioned)
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    Outcome::new(value, vec![]).into_cli_compatible_json()
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
#[path = "subagent_control_tests.rs"]
mod tests;
