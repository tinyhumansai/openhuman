//! Idle-only admission into the ordinary interruptible desktop turn lane.
use super::super::{
    run_task::run_chat_task,
    types::{ChatRequestMetadata, InFlightEntry},
};
use super::state::{in_flight, parallel_in_flight, key_for};
use crate::agent::{
    autonomous::{observation::Guard, service, store, types::Work},
    turn_origin::{AgentTurnOrigin, TrustedAutomationSource},
};
use crate::core::runtime::DispatchCtx;
use std::{path::PathBuf, sync::Arc};
use tinyagents_harness::run_queue::RunQueue;
use tokio_util::sync::CancellationToken;

pub(crate) async fn start_autonomous_turn(root: PathBuf, job: Work) -> anyhow::Result<()> {
    let ctx = DispatchCtx::ambient();
    let mut lane = in_flight(&ctx).lock_owned().await;
    // Keep unattended concurrency bounded, without capping the lifetime of a task.
    if service::suppressed(&job.thread_id)
        || lane.len() >= 2
        || lane.contains_key(&key_for(&ctx, &job.thread_id))
    {
        return Ok(());
    }
    if parallel_in_flight(&ctx)
        .lock_owned()
        .await
        .values()
        .any(|p| p.thread_id == job.thread_id)
    {
        return Ok(());
    }
    if crate::agent::orchestration::background_completions::has_pending_for_thread(&job.thread_id)
        || crate::agent::orchestration::running_subagents::has_running_for_thread(&job.thread_id)
    {
        return Ok(());
    }
    let request = format!("autonomous-{}", uuid::Uuid::new_v4());
    let claimed = match store::update(&root, &job.thread_id, &job.id, |w| {
        anyhow::ensure!(
            w.runnable() && w.next_run_ms <= service::now_ms(),
            "Task is no longer eligible"
        );
        w.lease = Some(request.clone());
        w.finish_requested = false;
        Ok(())
    })
    .await
    {
        Ok(work) => work,
        Err(_) => return Ok(()),
    };
    let cancel = CancellationToken::new();
    let run_queue = Arc::new(RunQueue::new());
    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
    let request_task = request.clone();
    let cancel_task = cancel.clone();
    let queue_task = run_queue.clone();
    let thread = job.thread_id.clone();
    let ctx_task = ctx.clone();
    let handle = tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
        async move {
            if start_rx.await.is_err() {
                return;
            }
            let observer = Guard::new(&request_task);
            let client = super::SYSTEM_CLIENT_ID;
            let origin = AgentTurnOrigin::TrustedAutomation {
                job_id: request_task.clone(),
                source: TrustedAutomationSource::AutonomousWork,
            };
            let prompt = claimed.prompt();
            let origin_copy = origin.clone();
            let approval = crate::security::approval::ApprovalChatContext {
                thread_id: claimed.thread_id.clone(),
                client_id: client.into(),
                request_id: Some(request_task.clone()),
            };
            let root_for_task = root.clone();
            let thread_for_task = claimed.thread_id.clone();
            let request_for_task = request_task.clone();
            let prompt_for_task = prompt.clone();
            let ctx_inner = ctx_task.clone();
            let result = super::turn_guards::run_turn_under_cancel_and_deadline(
                &ctx_task,
                cancel_task.clone(),
                origin_copy,
                approval,
                |host_env, activity, approval_turn| {
                    let thread_for_task = thread_for_task.clone();
                    let request_for_task = request_for_task.clone();
                    let prompt_for_task = prompt_for_task.clone();
                    let root_for_task = root_for_task.clone();
                    let queue_task = queue_task.clone();
                    let ctx_run = ctx_inner.clone();
                    let origin = origin.clone();
                    async move {
                        let current = crate::config::rpc::load_config_with_timeout().await?;
                        if current.workspace_dir != root_for_task {
                            return Err("Active user workspace changed; autonomous turn was not run".into());
                        }
                        let preferences =
                            crate::agent::autonomous::preferences::load(&root_for_task, &thread_for_task)
                                .await
                                .map_err(|e| format!("Could not restore continuation preferences: {e}"))?;
                        if let Some(selected) = &preferences {
                            crate::agent::tinyagents::apply_requested_effort(
                                &ctx_run,
                                &thread_for_task,
                                selected.reasoning.as_deref(),
                            )?;
                        }
                        run_chat_task(
                            &ctx_run,
                            client,
                            &thread_for_task,
                            &request_for_task,
                            &prompt_for_task,
                            preferences.as_ref().and_then(|p| p.model.clone()),
                            preferences.as_ref().and_then(|p| p.temperature),
                            None,
                            queue_task,
                            ChatRequestMetadata {
                                source: Some("autonomous".into()),
                                ..Default::default()
                            },
                            origin,
                            false,
                            host_env,
                            activity,
                            approval_turn,
                        )
                        .await
                    }
                },
            )
            .await;
            if let Some(result) = result {
                let error = result.as_ref().err().map(|e| {
                    let c = super::super::web_errors::classify_inference_error(e);
                    if e.contains("tool_call_recovery_exhausted") {
                        (e.clone(), true, None)
                    } else {
                        (c.message, c.retryable, c.retry_after_ms)
                    }
                });
                let settled =
                    service::settle(&root, &claimed, &request_task, &observer.snapshot(), error)
                        .await;
                match (result, settled) {
                    (Ok(chat), Ok(work)) if !cancel_task.is_cancelled() => {
                        let response = if work.phase
                            == crate::agent::autonomous::types::Phase::Complete
                        {
                            format!(
                                "Verified completion.\n\n{}\n\nVerification evidence:\n{}",
                                chat.full_response,
                                work.evidence.join("\n")
                            )
                        } else {
                            let mut sections = Vec::new();
                            sections.push(format!("Autonomous work: {:?}.", work.phase));
                            if !chat.full_response.trim().is_empty() {
                                sections.push(chat.full_response.trim().to_string());
                            }
                            if !work.checkpoint.trim().is_empty() {
                                sections.push(format!("Checkpoint: {}", work.checkpoint.trim()));
                            }
                            if let Some(ref err) = work.last_error {
                                sections.push(format!("Notice: {}", err));
                            }
                            sections.push(work.continuation_notice(service::now_ms()));
                            sections.join("\n\n")
                        };
                        super::super::presentation::deliver_response(
                            &ctx_task,
                            client,
                            &claimed.thread_id,
                            &request_task,
                            &response,
                            &prompt,
                            &chat.citations,
                            chat.usage.as_ref(),
                            Some(root.as_path()),
                            chat.timing,
                            false,
                        )
                        .await;
                    }
                    (Err(_), Ok(work)) if !cancel_task.is_cancelled() => {
                        super::super::presentation::deliver_response(
                            &ctx_task,
                            client,
                            &claimed.thread_id,
                            &request_task,
                            &format!(
                                "Autonomous work: {:?}. {}\n\n{}",
                                work.phase,
                                work.last_error.as_deref().unwrap_or("Work is incomplete."),
                                work.continuation_notice(service::now_ms())
                            ),
                            &prompt,
                            &[],
                            None,
                            Some(root.as_path()),
                            None,
                            false,
                        )
                        .await;
                    }
                    (_, Ok(_)) => {}
                    (_, Err(error)) => {
                        tracing::error!(error = %error, "[autonomous] could not settle task");
                    }
                }
            } else {
                if let Err(error) = store::pause(&root, &claimed.thread_id, None).await {
                    tracing::warn!(error = %error, "[autonomous] could not pause task on cancellation");
                }
            }
            let mut lane = in_flight(&ctx_task).lock_owned().await;
            if lane
                .get(&key_for(&ctx_task, &claimed.thread_id))
                .is_some_and(|e| e.request_id == request_task)
            {
                lane.remove(&key_for(&ctx_task, &claimed.thread_id));
            }
        },
    ));
    lane.insert(
        key_for(&ctx, &thread),
        InFlightEntry {
            request_id: request,
            client_id: super::SYSTEM_CLIENT_ID.into(),
            handle,
            run_queue,
            cancel_token: cancel,
        },
    );
    let _ = start_tx.send(());
    Ok(())
}

#[cfg(test)]
#[path = "autonomous_turn_tests.rs"]
mod tests;
