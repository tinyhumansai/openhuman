use super::*;
use crate::agent::orchestration::background_completions::CompletionTarget;
use crate::agent::subagent_host::{SubagentRunOutcome, SubagentRunStatus};
use crate::core::runtime::turn_origin::AgentTurnOrigin;
use crate::core::runtime::{CoreContext, DispatchCtx};
use crate::progress::AgentProgress;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) async fn report_spawned_progress(
    progress_sink: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    agent_id: String,
    task_id: String,
    prompt: String,
    worker_thread_id: Option<String>,
    display_name: String,
    parent_call_id: Option<String>,
) {
    if let Some(progress) = progress_sink {
        let _ = progress
            .send(AgentProgress::SubagentSpawned {
                agent_id,
                task_id,
                mode: "async".to_string(),
                dedicated_thread: worker_thread_id.is_some(),
                prompt_chars: prompt.chars().count(),
                prompt,
                worker_thread_id,
                display_name: Some(display_name),
                parent_call_id,
            })
            .await;
    }
}

async fn mark_driver_finished(
    store: &Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore>,
    session_id: &str,
    task_id: &str,
    status: &SubagentRunStatus,
    history: Vec<tinyagents_session::transcript::TranscriptMessage>,
) -> Result<(), String> {
    let status = match subagent_sessions::DurableSubagentStatus::from_run_status(status) {
        subagent_sessions::DurableSubagentStatus::Running => {
            tinyagents_orchestration::subagent::DurableSubagentStatus::Running
        }
        subagent_sessions::DurableSubagentStatus::Idle => {
            tinyagents_orchestration::subagent::DurableSubagentStatus::Idle
        }
        subagent_sessions::DurableSubagentStatus::AwaitingUser => {
            tinyagents_orchestration::subagent::DurableSubagentStatus::AwaitingUser
        }
        subagent_sessions::DurableSubagentStatus::Failed => {
            tinyagents_orchestration::subagent::DurableSubagentStatus::Failed
        }
        subagent_sessions::DurableSubagentStatus::Closed => {
            tinyagents_orchestration::subagent::DurableSubagentStatus::Closed
        }
    };
    let history = tinyagents_runtime::transcript::transcript_to_messages(&history);
    if store
        .mark_finished(session_id, task_id, status, history)
        .await
        .map_err(|error| error.to_string())?
    {
        Ok(())
    } else {
        Err("driver session no longer belongs to this task".into())
    }
}

async fn mark_driver_failed(
    store: &Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore>,
    session_id: &str,
    task_id: &str,
    error: String,
) -> Result<(), String> {
    if store
        .mark_failed(session_id, task_id, error)
        .await
        .map_err(|error| error.to_string())?
    {
        Ok(())
    } else {
        Err("driver session no longer belongs to this task".into())
    }
}

pub(super) fn make_completion(
    completion_core: Option<Arc<CoreContext>>,
    completion_origin: Option<AgentTurnOrigin>,
    progress_sink: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    background_workspace_dir: PathBuf,
    background_dispatch: DispatchCtx,
    background_agent_id: String,
    background_task_id: String,
    background_task_key: String,
    background_parent_session: String,
    completion_target: CompletionTarget,
    completion_worker_thread_id: Option<String>,
    background_store: Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore>,
    background_subagent_session_id: String,
    background_parent_thread_id: Option<String>,
    seed_len: usize,
) -> impl Fn(
    String,
    Result<SubagentRunOutcome, String>,
    bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
       + Send
       + Sync
       + 'static {
    move |_target: String,
          result: Result<SubagentRunOutcome, String>,
          emit_lifecycle_effects: bool|
          -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let core = completion_core.clone();
        let origin = completion_origin.clone();
        let progress_sink = progress_sink.clone();
        let background_workspace_dir = background_workspace_dir.clone();
        let background_dispatch = background_dispatch.clone();
        let background_agent_id = background_agent_id.clone();
        let background_task_id = background_task_id.clone();
        let background_task_key = background_task_key.clone();
        let background_parent_session = background_parent_session.clone();
        let completion_target = completion_target.clone();
        let background_worker_thread_id = completion_worker_thread_id.clone();
        let background_store = background_store.clone();
        let background_subagent_session_id = background_subagent_session_id.clone();
        let background_parent_thread_id = background_parent_thread_id.clone();
        let owner_dispatch = background_dispatch.clone();
        let owner_task_id = background_task_id.clone();
        Box::pin(async move {
            let keep_owner_active = matches!(
                &result,
                Ok(outcome)
                    if matches!(&outcome.status, SubagentRunStatus::AwaitingUser { .. })
            );
            let finish = async move {
                let mut abort_report =
                    AbortReport::arm(progress_sink, &background_agent_id, &background_task_id);
                let result = result.map_err(anyhow::Error::msg);
                match result {
                    Ok(outcome) => {
                        let status = if matches!(outcome.status, SubagentRunStatus::Completed)
                            && super::super::dispatch::is_unexecuted_tool_call_stub(&outcome.output)
                        {
                            SubagentRunStatus::Incomplete {
                                reason: "returned an unexecuted tool call instead of a result"
                                    .to_string(),
                                failure_cause: None,
                            }
                        } else {
                            outcome.status.clone()
                        };
                        match status.clone() {
                            SubagentRunStatus::Completed => {
                                if let Err(err) = mark_driver_finished(
                                    &background_store,
                                    &background_subagent_session_id,
                                    &outcome.task_id,
                                    &status,
                                    outcome.final_history.clone(),
                                )
                                .await
                                {
                                    log::warn!(
                                        "[subagent_reuse] mark_completed failed subagent_session_id={} task_id={} agent_id={} error={}",
                                        background_subagent_session_id,
                                        outcome.task_id,
                                        outcome.agent_id,
                                        err
                                    );
                                }
                                // A workflow proposal from the child's tool history is
                                // durable state, not prose: persist it into the parent chat
                                // thread (survives reload / reconnect) and carry the full
                                // payload in the delivery notice for the follow-up turn.
                                if emit_lifecycle_effects {
                                    let delivery_summary = attach_workflow_proposal(
                                        &background_dispatch,
                                        &background_workspace_dir,
                                        background_parent_thread_id.as_deref(),
                                        &outcome.task_id,
                                        &outcome.agent_id,
                                        this_run_history(&outcome.final_history, seed_len),
                                        outcome.output.clone(),
                                    );
                                    completion_target
                                        .completed(
                                            &outcome.task_id,
                                            &outcome.agent_id,
                                            delivery_summary,
                                        )
                                        .await;
                                    crate::agent::orchestration::subagent_events::publish_subagent_completed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                                    abort_report
                                        .deliver(AgentProgress::SubagentCompleted {
                                            agent_id: outcome.agent_id,
                                            task_id: outcome.task_id,
                                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                                            iterations: outcome.iterations as u32,
                                            output_chars: outcome.output.chars().count(),
                                            output: outcome.output.clone(),
                                            // Detached by construction: this tool
                                            // takes `detached_child()`, so this child's
                                            // spend never reached the parent turn's
                                            // ledger or `chat_done`. See the field docs.
                                            usage: Some(outcome.usage),
                                            worktree_path: None,
                                            changed_files: Vec::new(),
                                            dirty_status: None,
                                            stop: None,
                                        })
                                        .await;
                                }
                            }
                            SubagentRunStatus::Incomplete { ref reason, .. } => {
                                // Async sub-agent stopped short (stuck halt / iteration
                                // cap). Mark the session finished and deliver the PARTIAL
                                // progress back to the parent, framed so it is not
                                // mistaken for a completed result (#4096).
                                if let Err(err) = mark_driver_finished(
                                    &background_store,
                                    &background_subagent_session_id,
                                    &outcome.task_id,
                                    &status,
                                    outcome.final_history.clone(),
                                )
                                .await
                                {
                                    log::warn!(
                                        "[subagent_reuse] mark_incomplete failed subagent_session_id={} task_id={} agent_id={} error={}",
                                        background_subagent_session_id,
                                        outcome.task_id,
                                        outcome.agent_id,
                                        err
                                    );
                                }
                                let framed = format!(
                                    "[SUBAGENT_INCOMPLETE] the sub-agent {reason} and did not finish. \
                             Partial progress:\n{}",
                                    outcome.output
                                );
                                // An incomplete run may still have produced a full
                                // proposal before stalling — preserve it durably too.
                                if emit_lifecycle_effects {
                                    let framed = attach_workflow_proposal(
                                        &background_dispatch,
                                        &background_workspace_dir,
                                        background_parent_thread_id.as_deref(),
                                        &outcome.task_id,
                                        &outcome.agent_id,
                                        this_run_history(&outcome.final_history, seed_len),
                                        framed,
                                    );
                                    completion_target
                                        .completed(&outcome.task_id, &outcome.agent_id, framed)
                                        .await;
                                    crate::agent::orchestration::subagent_events::publish_subagent_completed(
                            background_parent_session,
                            outcome.task_id.clone(),
                            outcome.agent_id.clone(),
                            outcome.elapsed.as_millis() as u64,
                            outcome.output.chars().count(),
                            outcome.iterations,
                        );
                                    abort_report
                                        .deliver(AgentProgress::SubagentCompleted {
                                            agent_id: outcome.agent_id,
                                            task_id: outcome.task_id,
                                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                                            iterations: outcome.iterations as u32,
                                            output_chars: outcome.output.chars().count(),
                                            output: outcome.output.clone(),
                                            // Detached by construction: this tool
                                            // takes `detached_child()`, so this child's
                                            // spend never reached the parent turn's
                                            // ledger or `chat_done`. See the field docs.
                                            usage: Some(outcome.usage),
                                            worktree_path: None,
                                            changed_files: Vec::new(),
                                            dirty_status: None,
                                            stop: incomplete_stop(
                                                reason,
                                                outcome.failure_cause.clone(),
                                            ),
                                        })
                                        .await;
                                }
                            }
                            SubagentRunStatus::Cancelled => {
                                let reason = match background_store
                                    .get(&background_subagent_session_id)
                                    .await
                                {
                                    Ok(session) => session
                                        .filter(|session| {
                                            session.current_task_id.as_deref()
                                                == Some(outcome.task_id.as_str())
                                        })
                                        .and_then(|session| session.latest_error)
                                        .or_else(|| {
                                            crate::agent::orchestration::completion_owners::take_driver_cancel_reason(
                                                &background_dispatch,
                                                &outcome.task_id,
                                            )
                                        }),
                                    Err(error) => {
                                        log::warn!(
                                            "[subagent_reuse] cancellation reason lookup failed subagent_session_id={} task_id={} error={error}",
                                            background_subagent_session_id,
                                            outcome.task_id
                                        );
                                        None
                                    }
                                };
                                let error = match reason
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|reason| !reason.is_empty())
                                {
                                    Some(reason) => {
                                        format!("async sub-agent was cancelled. Reason: {reason}")
                                    }
                                    None => "async sub-agent was cancelled".to_string(),
                                };
                                if let Err(err) = mark_driver_finished(
                                    &background_store,
                                    &background_subagent_session_id,
                                    &outcome.task_id,
                                    &outcome.status,
                                    outcome.final_history.clone(),
                                )
                                .await
                                {
                                    log::warn!(
                                        "[subagent_reuse] mark_cancelled failed subagent_session_id={} task_id={} agent_id={} error={}",
                                        background_subagent_session_id,
                                        outcome.task_id,
                                        outcome.agent_id,
                                        err
                                    );
                                }
                                if emit_lifecycle_effects {
                                    completion_target
                                        .failed(&outcome.task_id, &outcome.agent_id, &error)
                                        .await;
                                    crate::agent::orchestration::subagent_events::publish_subagent_failed(
                                        background_parent_session,
                                        outcome.task_id.clone(),
                                        outcome.agent_id.clone(),
                                        error.clone(),
                                    );
                                    abort_report
                                        .deliver(AgentProgress::SubagentFailed {
                                            agent_id: outcome.agent_id,
                                            task_id: outcome.task_id,
                                            error,
                                        })
                                        .await;
                                }
                            }
                            SubagentRunStatus::AwaitingUser {
                                ref question,
                                ref checkpoint,
                                ..
                            } => {
                                if let Err(err) = mark_driver_finished(
                                    &background_store,
                                    &background_subagent_session_id,
                                    &outcome.task_id,
                                    &outcome.status,
                                    outcome.final_history.clone(),
                                )
                                .await
                                {
                                    log::warn!(
                                        "[subagent_reuse] mark_awaiting_user failed subagent_session_id={} task_id={} agent_id={} error={}",
                                        background_subagent_session_id,
                                        outcome.task_id,
                                        outcome.agent_id,
                                        err
                                    );
                                }
                                // #4896: a detached child that pauses for input won't continue
                                // on its own — queue a framed notice so the parent chat learns it
                                // needs input, not "Accepted". Same idle-gated path as a success.
                                if emit_lifecycle_effects {
                                    completion_target
                                        .awaiting_input(
                                            &outcome.task_id,
                                            &background_task_key,
                                            &outcome.agent_id,
                                            question,
                                            checkpoint.is_some(),
                                        )
                                        .await;
                                    crate::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
                                        background_parent_session,
                                        outcome.task_id.clone(),
                                        outcome.agent_id.clone(),
                                        question.clone(),
                                    );
                                    abort_report
                                        .deliver(AgentProgress::SubagentAwaitingUser {
                                            agent_id: outcome.agent_id,
                                            task_id: outcome.task_id,
                                            question: question.clone(),
                                            worker_thread_id: background_worker_thread_id.clone(),
                                            checkpoint_path: checkpoint
                                                .as_ref()
                                                .map(|path| path.to_string_lossy().to_string()),
                                        })
                                        .await;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        let error = err.to_string();
                        let failure = super::super::dispatch::format_subagent_failure(
                            "spawn_subagent",
                            &error,
                        );
                        if let Err(store_err) = mark_driver_failed(
                            &background_store,
                            &background_subagent_session_id,
                            &background_task_id,
                            error.clone(),
                        )
                        .await
                        {
                            log::warn!(
                                "[subagent_reuse] mark_failed failed subagent_session_id={} task_id={} agent_id={} error={}",
                                background_subagent_session_id,
                                background_task_id,
                                background_agent_id,
                                store_err
                            );
                        }
                        // #4896: a detached child that errors previously only
                        // published an event — nothing reached chat, so the parent
                        // turn finalized on "Accepted" and the failure was lost.
                        // Queue a framed failure notice so background_delivery
                        // surfaces it as a follow-up chat turn.
                        if emit_lifecycle_effects {
                            completion_target
                                .failed(&background_task_id, &background_agent_id, &failure)
                                .await;
                            crate::agent::orchestration::subagent_events::publish_subagent_failed(
                                background_parent_session,
                                background_task_id.clone(),
                                background_agent_id.clone(),
                                failure.clone(),
                            );
                            abort_report
                                .deliver(AgentProgress::SubagentFailed {
                                    agent_id: background_agent_id,
                                    task_id: background_task_id,
                                    error: failure,
                                })
                                .await;
                        }
                    }
                }
                abort_report.disarm();
            };
            match core {
                Some(core) => {
                    crate::core::runtime::CoreContext::scope_with_turn_origin(
                        core,
                        origin,
                        Box::pin(finish),
                    )
                    .await
                }
                None => finish.await,
            }
            if !keep_owner_active {
                crate::agent::orchestration::completion_owners::finish_driver_task(
                    &owner_dispatch,
                    &owner_task_id,
                );
            }
        })
    }
}
