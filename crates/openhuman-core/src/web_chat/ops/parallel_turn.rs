//! Forked (`QueueMode::Parallel`) turns: spawning one alongside whatever is
//! already in flight on a thread, and the request- or thread-scoped
//! cancellation paths that tear them down.

use std::sync::Arc;
use std::time::Duration;

use tinyagents_harness::run_queue::RunQueue;
use tokio_util::sync::CancellationToken;

use crate::core::socketio::WebChannelEvent;

use super::super::event_bus::publish_web_channel_event;
use super::super::run_task::run_chat_task;
use super::super::types::{ChatRequestMetadata, ParallelEntry};
use super::super::web_errors::classify_inference_error;
use super::state::PARALLEL_IN_FLIGHT;
use super::turn_guards::{
    run_turn_under_cancel_and_deadline, sentry_suppression_reason, timeout_bound_tag,
};

/// Spawn an independent, forked (`QueueMode::Parallel`) turn. It snapshots the
/// thread's history-at-start (inside `run_chat_task` with `fork = true`), runs
/// concurrently with any other turn on the thread, and on completion delivers
/// its response (append-only) and removes itself from `PARALLEL_IN_FLIGHT`.
/// Emits the same per-`request_id` stream events as a primary turn, so the UI
/// can render it as an interleaved branch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn spawn_parallel_turn(
    client_id: &str,
    thread_id: &str,
    request_id: String,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    metadata: ChatRequestMetadata,
) {
    let cancel_token = CancellationToken::new();
    let task_cancel_token = cancel_token.clone();

    let client_id_task = client_id.to_string();
    let thread_id_task = thread_id.to_string();
    let request_id_task = request_id.clone();
    let user_message = message.to_string();
    // Forked turns don't participate in the steer/followup/collect queue, but
    // `run_chat_task` requires a queue handle — give each its own.
    let run_queue = Arc::new(RunQueue::new());

    let handle = tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
        async move {
            let approval_ctx = crate::security::approval::ApprovalChatContext {
                thread_id: thread_id_task.clone(),
                client_id: client_id_task.clone(),
            };
            let origin = crate::agent::turn_origin::AgentTurnOrigin::WebChat {
                thread_id: thread_id_task.clone(),
                client_id: client_id_task.clone(),
                request_id: Some(request_id_task.clone()),
            };
            let result = run_turn_under_cancel_and_deadline(
                task_cancel_token,
                origin,
                approval_ctx,
                run_chat_task(
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &user_message,
                    model_override,
                    temperature,
                    locale,
                    run_queue,
                    metadata,
                    /* fork */ true,
                ),
            )
            .await;

            match result {
                Some(Ok(chat_result)) => {
                    crate::web_chat::presentation::deliver_response(
                        &client_id_task,
                        &thread_id_task,
                        &request_id_task,
                        &chat_result.full_response,
                        &user_message,
                        &chat_result.citations,
                        chat_result.usage.as_ref(),
                        // The workspace the turn ran in, so the reply is stored
                        // there before it is announced (#6034).
                        Some(chat_result.workspace_dir.as_path()),
                    )
                    .await;
                }
                Some(Err(err)) => {
                    log::warn!(
                        "[web-channel] parallel run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                        client_id_task,
                        thread_id_task,
                        request_id_task,
                        err
                    );
                    let detailed = format!(
                        "parallel run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                        client_id_task, thread_id_task, request_id_task, err
                    );
                    let classified = classify_inference_error(&err);
                    let classified_type = classified.error_type;

                    // A parallel turn runs under the same deadline wrapper as the
                    // serial one and dies the same way, but this branch reported
                    // NOTHING to Sentry — not merely the timeouts this PR
                    // un-suppresses, but every error type, since the parallel path
                    // was added. So a discarded turn was invisible here even
                    // before the suppression arm existed, and fixing only
                    // `start_chat` would have left `QueueMode::Parallel` exactly
                    // as blind as it was (#5804 review).
                    //
                    // Same policy as the serial site, deliberately sharing
                    // `sentry_suppression_reason` rather than restating it: the
                    // outer backstop stays suppressed, a harness `Timeout` reports
                    // with the ceiling that fired.
                    if let Some(reason) = sentry_suppression_reason(&detailed) {
                        log::info!(
                            target: "web_channel",
                            "[web_channel.spawn_parallel_turn] suppressed Sentry emission for {} \
                             client_id={} thread_id={} request_id={} error_type={} message={}",
                            reason,
                            client_id_task,
                            thread_id_task,
                            request_id_task,
                            classified_type,
                            detailed
                        );
                    } else {
                        crate::core::observability::report_error_or_expected(
                            detailed.as_str(),
                            "web_channel",
                            "spawn_parallel_turn",
                            &[
                                ("channel", "web"),
                                ("error_type", classified_type),
                                ("thread_id", thread_id_task.as_str()),
                                ("request_id", request_id_task.as_str()),
                                ("queue_mode", "parallel"),
                                ("timeout_bound", timeout_bound_tag(&detailed)),
                            ],
                        );
                    }

                    publish_web_channel_event(WebChannelEvent {
                        event: "chat_error".to_string(),
                        client_id: client_id_task.clone(),
                        thread_id: thread_id_task.clone(),
                        request_id: request_id_task.clone(),
                        message: Some(classified.message),
                        error_type: Some(classified.error_type.to_string()),
                        error_source: Some(classified.source.to_string()),
                        error_retryable: Some(classified.retryable),
                        error_retry_after_ms: classified.retry_after_ms,
                        error_provider: classified.provider,
                        error_fallback_available: classified.fallback_available,
                        ..Default::default()
                    });
                }
                None => {
                    log::info!(
                        "[web-channel] parallel turn cancelled cooperatively thread_id={} request_id={}",
                        thread_id_task,
                        request_id_task
                    );
                }
            }

            PARALLEL_IN_FLIGHT.lock().await.remove(&request_id_task);
        },
    ));

    PARALLEL_IN_FLIGHT.lock().await.insert(
        request_id,
        ParallelEntry {
            thread_id: thread_id.to_string(),
            handle,
            cancel_token,
        },
    );
}

/// Cooperatively cancel every parallel turn on a thread. Returns the cancelled
/// request ids. Used by the thread-level cancel paths so a cancel/stop also
/// tears down any concurrent forked turns, not just the primary turn.
pub(crate) async fn cancel_parallel_turns_for_thread(thread_id: &str) -> Vec<String> {
    let mut cancelled = Vec::new();
    let mut parallel = PARALLEL_IN_FLIGHT.lock().await;
    let request_ids: Vec<String> = parallel
        .iter()
        .filter(|(_, entry)| entry.thread_id == thread_id)
        .map(|(request_id, _)| request_id.clone())
        .collect();
    for request_id in request_ids {
        if let Some(entry) = parallel.remove(&request_id) {
            entry.cancel_token.cancel();
            let mut handle = entry.handle;
            tokio::spawn(async move {
                tokio::select! {
                    _ = &mut handle => {}
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        handle.abort();
                    }
                }
            });
            cancelled.push(request_id);
        }
    }
    cancelled
}

/// Cancel a single parallel (forked) turn identified by `request_id`, but only
/// when it belongs to `thread_id`. Returns the cancelled id (as a one-element
/// vec, mirroring [`cancel_parallel_turns_for_thread`]) or empty when no such
/// parallel turn exists. Request-scoped cancel path (#4760).
pub(crate) async fn cancel_parallel_turn_by_request_id(
    thread_id: &str,
    request_id: &str,
) -> Vec<String> {
    let mut parallel = PARALLEL_IN_FLIGHT.lock().await;
    let matches = parallel
        .get(request_id)
        .map(|entry| entry.thread_id == thread_id)
        .unwrap_or(false);
    if !matches {
        return Vec::new();
    }
    if let Some(entry) = parallel.remove(request_id) {
        entry.cancel_token.cancel();
        let mut handle = entry.handle;
        tokio::spawn(async move {
            tokio::select! {
                _ = &mut handle => {}
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    handle.abort();
                }
            }
        });
        return vec![request_id.to_string()];
    }
    Vec::new()
}
