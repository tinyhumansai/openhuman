//! The web chat's delivery ports for the turn runner: where a turn's progress
//! and its single terminal event go ([`WebEvents`]), how a failure reads to
//! the user ([`WebFailures`]), where a finished reply is stored
//! ([`WebReplies`]) and how a stopped turn is closed ([`WebSummarizer`]).
//!
//! The runner persists the reply, then releases the thread, then emits the
//! terminal event, so a `chat_done` is never the signal that lets the next
//! message interrupt the turn that just finished.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tinyagents_harness::progress::AgentProgress;
use tinyagents_harness::turn_runner::{
    EventSink, FailureClassification, FailureClassifier, ReplyStore, Summarizer, TerminalEvent,
    TurnRequest, TurnResult,
};
use tinychannels_bus::web as bus_web;

use super::runner::{CancelNote, Shared};
use super::surface::{Surface, TurnIds};
use super::turn_close::{close_reply, StopCause, StoppedTurn};
use super::web_errors::{
    classify_inference_error, is_outer_backstop_timeout, turn_idle_error_message,
    turn_timeout_error_message, ClassifiedError,
};
use crate::agent::queued_turn::QueuedPayload;

/// The runner's wall-clock failure, as the marker string the classifier and the
/// Sentry policy recognise.
const RUNNER_DEADLINE: &str = "turn wall-clock deadline";
/// The runner's idle-guard failure.
const RUNNER_IDLE: &str = "turn idle timeout";

/// Reason a terminal `run_chat_task` error should be kept OUT of Sentry, or
/// `None` when it is a genuine defect that must page.
///
/// A suppressed case is a deterministic, user-surfaced, retryable agent-loop
/// outcome (a terminal `chat_error` already reaches the client, so a Sentry
/// event is noise): the max-iteration cap, and the **outer** web-turn
/// wall-clock backstop (issue #4746), where the turn wedged outside the harness
/// and produced no terminal event.
///
/// **Not suppressed: the harness's own `Timeout` (#5804).** It fires while
/// bounding a real model or tool call, so the run spent its budget doing work
/// and every result that work produced is discarded with the turn. That is a
/// defect signal, and suppressing it is what made the discarded-turn failure
/// invisible. The user-facing classification is untouched: both render the
/// graceful `turn_timeout` copy. Only the telemetry decision splits.
///
/// Pure over the formatted error string so the policy is unit-testable.
pub(crate) fn sentry_suppression_reason(detailed: &str) -> Option<&'static str> {
    if tinyagents_harness::agent_error::is_max_iterations_error(detailed) {
        Some("max-iteration cap")
    } else if is_outer_backstop_timeout(detailed) {
        Some("turn wall-clock backstop (no terminal event)")
    } else {
        None
    }
}

/// Which wall-clock bound a reported timeout hit, as a Sentry tag value. The
/// crate names the bound in the message (`RUN_BOUND_LABEL` vs
/// `PER_CALL_BOUND_LABEL`), and the two are different triage paths: a run that
/// spent its whole budget doing real work is a capacity problem, while one call
/// that blew a per-call ceiling is a wedged provider (#5804).
pub(crate) fn timeout_bound_tag(detailed: &str) -> &'static str {
    if let Some(bound) = tinyagents_harness::agent_error::TurnTimeoutBound::from_message(detailed) {
        bound.tag()
    } else if super::web_errors::is_turn_timeout_error(detailed) {
        "unclassified_timeout"
    } else {
        "none"
    }
}

/// The error a runner failure stands for: its two synthetic guard messages
/// become the marker strings the classifier knows, with the window that fired.
fn host_error(error: &str, request: Option<&TurnRequest<QueuedPayload>>) -> String {
    let secs = |window: Option<Duration>| window.map_or(0, |window| window.as_secs());
    match error {
        RUNNER_DEADLINE => turn_timeout_error_message(secs(request.and_then(|r| r.deadline))),
        RUNNER_IDLE => turn_idle_error_message(secs(request.and_then(|r| r.idle_timeout))),
        other => other.to_owned(),
    }
}

fn ids_of(request: &TurnRequest<QueuedPayload>) -> TurnIds {
    TurnIds {
        client_id: request.payload.client_id.clone(),
        thread_id: request.thread_id.clone(),
        request_id: request.request_id.clone(),
    }
}

/// Failure copy for the runner: the classification ladder of
/// `web_errors`, run on the error the host or the runner reported.
pub(super) struct WebFailures;

impl FailureClassifier for WebFailures {
    fn classify(&self, error: &str) -> (String, String) {
        let classified = classify_inference_error(&host_error(error, None));
        (classified.error_type.to_owned(), classified.message)
    }

    fn classify_failure(&self, error: &str) -> FailureClassification {
        let classified = classify_inference_error(&host_error(error, None));
        FailureClassification {
            class: classified.error_type.to_owned(),
            message: classified.message,
            details: Some(super::failure_details::encode(
                Some(classified.copy_key.to_owned()),
                classified.copy_params,
                error,
            )),
        }
    }
}

/// Stores the reply a turn finished with before it is announced (#6034): once
/// the row is on disk, a `chat_done` that is never delivered or painted costs
/// the user a repaint, not the reply.
pub(super) struct WebReplies {
    shared: Arc<Shared>,
}

impl WebReplies {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

#[async_trait]
impl ReplyStore<QueuedPayload> for WebReplies {
    async fn persist(
        &self,
        request: &TurnRequest<QueuedPayload>,
        _reply_id: &str,
        result: &TurnResult,
    ) -> Result<(), String> {
        let Some(ctx) = request.payload.ctx.as_ref() else {
            return Ok(());
        };
        let Some(done) = self
            .shared
            .with(&request.request_id, |slot| slot.result.clone())
        else {
            log::debug!(
                "[web-channel] no turn result to persist thread_id={} request_id={}",
                request.thread_id,
                request.request_id
            );
            return Ok(());
        };
        super::presentation::persist_reply(
            ctx,
            &request.thread_id,
            &request.request_id,
            &result.reply,
            &done.citations,
            Some(done.workspace_dir.as_path()),
        )
        .await;
        Ok(())
    }
}

/// Closes a turn a guard stopped, from what it got done and where it stuck.
pub(super) struct WebSummarizer {
    shared: Arc<Shared>,
}

impl WebSummarizer {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

#[async_trait]
impl Summarizer<QueuedPayload> for WebSummarizer {
    async fn grounded_close(
        &self,
        request: &TurnRequest<QueuedPayload>,
        reason: &str,
    ) -> Option<TurnResult> {
        let ctx = request.payload.ctx.clone()?;
        let activity = self
            .shared
            .with(&request.request_id, |slot| slot.activity.clone());
        let stuck = activity.stuck_point();
        let cause = if reason.contains("idle") {
            let idle = request
                .idle_timeout
                .unwrap_or_else(super::turn_activity::configured_idle_window);
            log::warn!(
                "[web-channel] turn idle guard fired: no progress for {}s with no model or tool call in flight; stuck={stuck:?}",
                idle.as_secs()
            );
            StopCause::Idle(idle)
        } else {
            let ceiling = request.deadline.unwrap_or_default();
            log::warn!(
                "[web-channel] turn wall-clock backstop fired after {}s with no terminal event (the harness wind-down did not close the turn in time); stuck={stuck:?} (issue #4746)",
                ceiling.as_secs()
            );
            StopCause::Ceiling(ceiling)
        };
        let (config, locale) = activity.turn_context();
        let closed = close_reply(StoppedTurn {
            thread_id: &request.thread_id,
            request_id: &request.request_id,
            cause,
            stuck,
            config,
            locale,
            ctx,
        })
        .await?;
        let reply = closed.full_response.clone();
        self.shared
            .with(&request.request_id, |slot| slot.result = Some(closed));
        Some(TurnResult {
            reply,
            data: serde_json::Value::Null,
        })
    }
}

/// Delivers a turn's progress, and its one terminal event, to the web surface.
pub(super) struct WebEvents {
    shared: Arc<Shared>,
}

impl WebEvents {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    /// The surface of the turn: the one its progress was projected on (so `seq`
    /// stays one sequence), or a fresh one for a turn that never started.
    async fn surface_of(
        &self,
        request: &TurnRequest<QueuedPayload>,
        projection: &Option<Arc<tokio::sync::Mutex<super::progress::ProgressProjection>>>,
    ) -> Option<Surface> {
        match projection {
            Some(projection) => Some(projection.lock().await.surface()),
            None => {
                let ctx = request.payload.ctx.as_ref()?;
                Some(Surface::host_only(ctx, ids_of(request)))
            }
        }
    }

    async fn completed(
        &self,
        request: &TurnRequest<QueuedPayload>,
        surface: &Surface,
        done: Option<super::types::WebChatTaskResult>,
    ) {
        let Some(done) = done else {
            log::warn!(
                "[web-channel] turn completed without a result thread_id={} request_id={}",
                request.thread_id,
                request.request_id
            );
            return;
        };
        surface
            .event(super::presentation::announced_chat_done(
                surface.context(),
                &done.full_response,
                &done.citations,
                done.usage.as_ref(),
                done.timing,
            ))
            .await;
        // The main single-user turn is the only surface with a human waiting
        // on a next-message suggestion (C5); a parallel fork has none.
        if request.disposition != tinyagents_harness::turn_runner::QueueDisposition::Parallel {
            if let Some(ctx) = request.payload.ctx.clone() {
                super::suggestions::spawn_follow_up_suggestions(
                    ctx,
                    request.payload.client_id.clone(),
                    request.thread_id.clone(),
                    request.request_id.clone(),
                    request.text.clone(),
                    done.full_response,
                );
            }
        }
    }

    async fn failed(
        &self,
        request: &TurnRequest<QueuedPayload>,
        surface: &Surface,
        error: &str,
        structured: Option<&FailureClassification>,
    ) {
        #[cfg(test)]
        super::ops::set_last_run_chat_task_error_for_test(Some(error)).await;
        let parallel =
            request.disposition == tinyagents_harness::turn_runner::QueueDisposition::Parallel;
        let raw = host_error(error, Some(request));
        let (client_id, thread_id, request_id) = (
            &request.payload.client_id,
            &request.thread_id,
            &request.request_id,
        );
        log::warn!(
            "[web-channel] {}run_chat_task failed client_id={client_id} thread_id={thread_id} request_id={request_id} error={raw}",
            if parallel { "parallel " } else { "" }
        );
        let detailed = format!(
            "{}run_chat_task failed client_id={client_id} thread_id={thread_id} request_id={request_id} error={raw}",
            if parallel { "parallel " } else { "" }
        );
        let classified = classify_inference_error(&raw);
        report_failure(&detailed, &classified, parallel, request);
        surface
            .event(chat_error_event(surface.context(), classified, structured))
            .await;
    }

    async fn cancelled(
        &self,
        request: &TurnRequest<QueuedPayload>,
        surface: &Surface,
        note: CancelNote,
    ) {
        let mut context = surface.context();
        if let Some(client_id) = note.client_id {
            context.client_id = client_id;
        }
        let (message, reason) = if note.user_stop {
            ("Cancelled", "user_stop")
        } else {
            ("Cancelled by newer request", "superseded")
        };
        log::info!(
            "[web-channel] turn cancelled thread_id={} request_id={} reason={reason}",
            request.thread_id,
            request.request_id
        );
        // `chat_cancelled` is the structured successor to
        // `chat_error{error_type:"cancelled"}`, kept alongside it for one
        // release so an older frontend build still resolves the turn.
        surface
            .event(bus_web::WebChannelEvent::ChatError {
                context: context.clone(),
                message: Some(message.to_owned()),
                error_type: Some("cancelled".to_owned()),
                error_source: None,
                error_retryable: None,
                error_retry_after_ms: None,
                error_provider: None,
                error_fallback_available: None,
                copy_key: None,
                copy_params: None,
                guardrail: None,
            })
            .await;
        surface
            .event(bus_web::WebChannelEvent::ChatCancelled {
                context,
                cancel_reason: Some(reason.to_owned()),
                superseded_by: note.superseded_by,
            })
            .await;
    }
}

/// Report a failed turn to Sentry, unless it is a deterministic, user-surfaced
/// outcome (see [`sentry_suppression_reason`]).
fn report_failure(
    detailed: &str,
    classified: &ClassifiedError,
    parallel: bool,
    request: &TurnRequest<QueuedPayload>,
) {
    let operation = if parallel {
        "spawn_parallel_turn"
    } else {
        "run_chat_task"
    };
    if let Some(reason) = sentry_suppression_reason(detailed) {
        log::info!(
            target: "web_channel",
            "[web_channel.{operation}] suppressed Sentry emission for {reason} client_id={} thread_id={} request_id={} error_type={} message={detailed}",
            request.payload.client_id,
            request.thread_id,
            request.request_id,
            classified.error_type,
        );
        return;
    }
    let mut tags = vec![
        ("channel", "web"),
        ("error_type", classified.error_type),
        ("thread_id", request.thread_id.as_str()),
        ("request_id", request.request_id.as_str()),
        // Names which ceiling fired for the harness timeouts; "none" for every
        // other error type (#5804).
        ("timeout_bound", timeout_bound_tag(detailed)),
    ];
    if parallel {
        tags.push(("queue_mode", "parallel"));
    }
    crate::core::observability::report_error_or_expected(detailed, "web_channel", operation, &tags);
}

fn chat_error_event(
    context: bus_web::WebEventContext,
    classified: ClassifiedError,
    structured: Option<&FailureClassification>,
) -> bus_web::WebChannelEvent {
    bus_web::WebChannelEvent::ChatError {
        context,
        message: Some(
            structured
                .map(|failure| failure.message.clone())
                .unwrap_or(classified.message),
        ),
        error_type: Some(
            structured
                .map(|failure| failure.class.as_str())
                .unwrap_or(classified.error_type)
                .to_owned(),
        ),
        error_source: Some(classified.source.to_owned()),
        error_retryable: Some(classified.retryable),
        error_retry_after_ms: classified.retry_after_ms,
        error_provider: classified.provider,
        error_fallback_available: classified.fallback_available,
        copy_key: structured.map_or_else(
            || Some(classified.copy_key.to_owned()),
            super::failure_details::copy_key,
        ),
        copy_params: structured.map_or(classified.copy_params, |failure| {
            super::failure_details::copy_params(failure)
        }),
        guardrail: None,
    }
}

#[async_trait]
impl EventSink<QueuedPayload> for WebEvents {
    async fn progress(&self, request: &TurnRequest<QueuedPayload>, event: AgentProgress) {
        let projection = self
            .shared
            .with(&request.request_id, |slot| slot.projection.clone());
        if let Some(projection) = projection {
            projection.lock().await.on_event(event).await;
        }
    }

    async fn terminal(&self, request: &TurnRequest<QueuedPayload>, event: TerminalEvent) {
        self.terminal_with_classification(request, event, None)
            .await;
    }

    async fn classified_failure(
        &self,
        request: &TurnRequest<QueuedPayload>,
        event: TerminalEvent,
        classification: FailureClassification,
    ) {
        self.terminal_with_classification(request, event, Some(classification))
            .await;
    }

    async fn collected(
        &self,
        request: &TurnRequest<QueuedPayload>,
        items: Vec<TurnRequest<QueuedPayload>>,
    ) {
        // Messages queued to be collected at the turn boundary that the turn
        // never absorbed start as follow-ups, so none is lost.
        if items.is_empty() {
            return;
        }
        let Some(ctx) = request.payload.ctx.clone() else {
            return;
        };
        log::info!(
            "[web-channel] dispatching {} collected message(s) thread_id={}",
            items.len(),
            request.thread_id
        );
        let shared = self.shared.clone();
        let spawner = ctx.clone();
        spawner.spawn(async move {
            for item in items {
                shared.forget_queued(&item.request_id);
                if let Err(err) = super::ops::start_chat(
                    &ctx,
                    &item.payload.client_id,
                    &item.thread_id,
                    &item.text,
                    item.payload.model_override.clone(),
                    item.payload.temperature,
                    item.payload.locale.clone(),
                    Some("followup".to_owned()),
                    item.payload.metadata.clone(),
                )
                .await
                {
                    log::warn!(
                        "[web-channel] failed to dispatch collected message thread_id={} err={err}",
                        item.thread_id
                    );
                }
            }
        });
    }
}

impl WebEvents {
    async fn terminal_with_classification(
        &self,
        request: &TurnRequest<QueuedPayload>,
        event: TerminalEvent,
        classification: Option<FailureClassification>,
    ) {
        let slot = self.shared.take(&request.request_id);
        // A message that waited in a run queue and never started was never
        // announced as a turn, so its being dropped is silent too.
        let never_started = self.shared.was_queued(&request.request_id);
        let (projection, done, note, _session_lease, _profile_turn) = match slot {
            Some(slot) => (
                slot.projection,
                slot.result,
                slot.cancel,
                slot.session_lease,
                slot.profile_turn,
            ),
            None => (None, None, None, None, None),
        };
        if never_started && !matches!(event, TerminalEvent::Completed { .. }) {
            log::debug!(
                "[web-channel] queued message dropped before it started thread_id={} request_id={}",
                request.thread_id,
                request.request_id
            );
            return;
        }
        if let Some(surface) = self.surface_of(request, &projection).await {
            match event {
                TerminalEvent::Completed { .. } => self.completed(request, &surface, done).await,
                TerminalEvent::Failed { message, .. } => {
                    // The runner no longer carries the raw error on the event;
                    // the classifier recorded it in the failure details.
                    let error = classification
                        .as_ref()
                        .and_then(super::failure_details::raw_error)
                        .unwrap_or(message);
                    self.failed(request, &surface, &error, classification.as_ref())
                        .await;
                }
                TerminalEvent::Cancelled { .. } => {
                    self.cancelled(request, &surface, note.unwrap_or_default())
                        .await;
                }
            }
        }
        // The projection's trace export is best-effort I/O and must not hold
        // up terminal delivery.
        if let (Some(projection), Some(ctx)) = (projection, request.payload.ctx.clone()) {
            ctx.spawn(async move {
                projection.lock().await.finish().await;
            });
        }
    }
}

#[cfg(test)]
#[path = "turn_events_tests.rs"]
mod tests;
