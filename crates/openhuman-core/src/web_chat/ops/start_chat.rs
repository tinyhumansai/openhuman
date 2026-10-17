//! The primary (non-parallel) turn entry point: ingress validation and
//! attachment preprocessing, prompt-injection and approval-reply routing,
//! run-queue dispatch, and the spawned turn body that delivers a response or
//! classifies and reports a failure.

use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::run_queue::{QueueLane, RunQueue};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::core::events::DomainEvent;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::web_chat::WebChannelEvent;

use super::super::event_bus::publish_web_channel_event_in;
use super::super::run_task::run_chat_task;
use super::super::types::{ChatRequestMetadata, InFlightEntry, QueueMode};
use super::super::web_errors::classify_inference_error;
use super::followups::dispatch_followups;
use super::parallel_turn::spawn_parallel_turn;
use super::state::{cancel_in_flight_gracefully, in_flight, key_for};
use super::turn_guards::{
    run_turn_under_cancel_and_deadline, sentry_suppression_reason, timeout_bound_tag,
};

/// `start_chat`'s error type.
///
/// `Guardrail` is a structured verdict from the prompt-injection/security
/// guardrail (`security::prompt_injection::enforce_prompt_input`) — the
/// frontend classifies on this variant (`chat_error.error_type == "guardrail"`
/// and a `guardrail` payload) instead of pattern-matching the user-facing
/// message string. Every other rejection (validation, a configured
/// `beforeSubmitPrompt` hook block, an approval-routing failure) stays
/// `Other`, which `Display`s exactly like the plain `String` errors this
/// replaced — existing `.to_string()` / `{err}` call sites need no other
/// change.
#[derive(Debug, Clone)]
pub enum StartChatError {
    Guardrail {
        verdict: String,
        score: f64,
        reasons: Vec<crate::web_chat::GuardrailReason>,
    },
    Other(String),
}

impl std::fmt::Display for StartChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The same user-facing copy `prompt_guard_user_message` gives a
            // fresh rejection — a caller that only has `.to_string()` (a
            // plain-string RPC error, a `{err}` log line) still gets an
            // actionable message, not a bare verdict/score dump.
            StartChatError::Guardrail { verdict, .. } => {
                f.write_str(guardrail_verdict_user_message(verdict))
            }
            StartChatError::Other(message) => write!(f, "{message}"),
        }
    }
}

/// User-facing copy for a guardrail verdict string (`"block"` /
/// `"review_blocked"` / `"allow"` — see the `match` in [`start_chat`] that
/// builds [`StartChatError::Guardrail`]). Shared by `Display` above so a
/// plain-string consumer of the error still reads the same rejection copy
/// [`prompt_guard_user_message`] gives a fresh (non-error-wrapped) decision.
fn guardrail_verdict_user_message(verdict: &str) -> &'static str {
    match verdict {
        "block" => prompt_guard_user_message(PromptEnforcementAction::Blocked),
        "review_blocked" => prompt_guard_user_message(PromptEnforcementAction::ReviewBlocked),
        _ => prompt_guard_user_message(PromptEnforcementAction::Allow),
    }
}

impl std::error::Error for StartChatError {}

impl From<String> for StartChatError {
    fn from(message: String) -> Self {
        StartChatError::Other(message)
    }
}

impl From<&str> for StartChatError {
    fn from(message: &str) -> Self {
        StartChatError::Other(message.to_string())
    }
}

/// Sentinel prefix a JSON-RPC/string-error caller can match on to recover the
/// structured guardrail verdict, the same pattern as
/// `core::observability::BACKEND_UNAVAILABLE_PREFIX`: the RPC surface only
/// carries `Result<_, String>`, so the socket path's `chat_error.guardrail`
/// payload gets a string-shaped equivalent here rather than a second, looser
/// error shape.
pub const GUARDRAIL_ERROR_PREFIX: &str = "GUARDRAIL:";

impl From<StartChatError> for String {
    fn from(error: StartChatError) -> Self {
        match error {
            StartChatError::Guardrail {
                verdict,
                score,
                reasons,
            } => {
                let payload = crate::web_chat::GuardrailPayload {
                    verdict,
                    score,
                    reasons,
                };
                format!(
                    "{GUARDRAIL_ERROR_PREFIX}{}",
                    serde_json::to_string(&payload).unwrap_or_default()
                )
            }
            StartChatError::Other(message) => message,
        }
    }
}

fn prompt_guard_user_message(action: PromptEnforcementAction) -> &'static str {
    match action {
        PromptEnforcementAction::Allow => "Message accepted.",
        PromptEnforcementAction::Blocked => {
            "Your message was blocked by a security policy. Please rephrase and remove instruction-override or secret-exfiltration requests."
        }
        PromptEnforcementAction::ReviewBlocked => {
            "Your message was flagged for security review and was not processed. Please rephrase the request in a direct, task-focused way."
        }
    }
}

/// Returned boxed and `#[inline(never)]` on purpose: other crates await this
/// (`openhuman-rpc`, `openhuman-embed`), and an `async fn` body is otherwise
/// re-instantiated inside every calling crate's state machine. Boxing here
/// keeps one copy, compiled in this crate.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub fn start_chat<'a>(
    ctx: &'a crate::core::runtime::DispatchCtx,
    client_id: &'a str,
    thread_id: &'a str,
    message: &'a str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    queue_mode: Option<String>,
    metadata: ChatRequestMetadata,
) -> futures::future::BoxFuture<'a, Result<String, StartChatError>> {
    Box::pin(start_chat_inner(
        ctx,
        client_id,
        thread_id,
        message,
        model_override,
        temperature,
        locale,
        queue_mode,
        metadata,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn start_chat_inner(
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    queue_mode: Option<String>,
    metadata: ChatRequestMetadata,
) -> Result<String, StartChatError> {
    let client_id = client_id.trim().to_string();
    let thread_id = thread_id.trim().to_string();
    let message = message.trim().to_string();

    if client_id.is_empty() {
        return Err(StartChatError::Other("client_id is required".to_string()));
    }
    if thread_id.is_empty() {
        return Err(StartChatError::Other("thread_id is required".to_string()));
    }
    if message.is_empty() {
        return Err(StartChatError::Other("message is required".to_string()));
    }
    // A profile this node no longer hosts (its lease was lost or released)
    // starts no new turn here. No-op outside SaaS.
    crate::profiles::host::ensure_hosted(ctx).map_err(StartChatError::Other)?;

    // Save originals before scanning, history, memory, or queue persistence.
    // A missing config/root cannot safely accept an upload.
    let mut message = if message.contains("[FILE:") || message.contains("[IMAGE:") {
        let config = ctx.load_config().await.map_err(|error| {
            StartChatError::Other(format!("upload configuration unavailable: {error}"))
        })?;
        // Stage where the turn will read: the thread's working folder when
        // one is bound (`run_task` applies the same folder), else action_dir.
        let workspace = crate::agent::attachments::thread_action_dir(
            ctx,
            &config,
            &config.workspace_dir,
            Some(&thread_id),
        )
        .await
        .map_err(|error| StartChatError::Other(format!("upload could not be saved: {error}")))?;
        crate::agent::attachments::stage(
            &message,
            &thread_id,
            &config,
            &crate::agent::attachments::AttachmentAccessScope {
                external_channel: false,
                workspace: Some(workspace),
            },
        )
        .await
        .map_err(|error| StartChatError::Other(format!("upload could not be saved: {error}")))?
    } else {
        message
    };

    let request_id = Uuid::new_v4().to_string();
    let prompt_decision = enforce_prompt_input(
        &message,
        PromptEnforcementContext {
            source: "web_chat.start_chat",
            request_id: Some(&request_id),
            user_id: Some(&client_id),
            session_id: Some(&thread_id),
        },
    );
    if !matches!(prompt_decision.action, PromptEnforcementAction::Allow) {
        log::warn!(
            "[web-channel] prompt rejected client_id={} thread_id={} request_id={} action={} score={:.2} reasons={} hash={} chars={}",
            client_id,
            thread_id,
            request_id,
            match prompt_decision.action {
                PromptEnforcementAction::Allow => "allow",
                PromptEnforcementAction::Blocked => "block",
                PromptEnforcementAction::ReviewBlocked => "review_blocked",
            },
            prompt_decision.score,
            prompt_decision
                .reasons
                .iter()
                .map(|r| r.code.as_str())
                .collect::<Vec<_>>()
                .join(","),
            prompt_decision.prompt_hash,
            prompt_decision.prompt_chars,
        );
        let verdict = match prompt_decision.action {
            PromptEnforcementAction::Allow => "allow",
            PromptEnforcementAction::Blocked => "block",
            PromptEnforcementAction::ReviewBlocked => "review_blocked",
        }
        .to_string();
        return Err(StartChatError::Guardrail {
            verdict,
            score: prompt_decision.score as f64,
            reasons: prompt_decision
                .reasons
                .iter()
                .map(|r| crate::web_chat::GuardrailReason {
                    code: r.code.clone(),
                    message: r.message.clone(),
                })
                .collect(),
        });
    }

    // Chat-native approval: if this thread has a parked approval and the message
    // is a yes/no reply, route it to the gate rather than starting a new turn.
    if let Some(gate) = crate::security::approval::ApprovalGate::try_global() {
        if let Some(request_id) = gate.pending_for_thread(&thread_id) {
            if let Some(decision) = crate::security::approval::parse_approval_reply(&message) {
                match gate.decide(&request_id, decision) {
                    Ok(Some(_)) => {
                        log::info!(
                            "[web-channel] routed chat reply to approval gate thread_id={} request_id={} decision={}",
                            thread_id,
                            request_id,
                            decision.as_str()
                        );
                        return Ok(request_id);
                    }
                    Ok(None) => {
                        log::warn!(
                            "[web-channel] approval reply targeted a non-pending/already-decided request thread_id={} request_id={} decision={} — dispatching as fresh turn",
                            thread_id,
                            request_id,
                            decision.as_str()
                        );
                    }
                    Err(err) => {
                        log::warn!(
                            "[web-channel] failed to route chat reply to approval gate thread_id={} request_id={} decision={} err={}",
                            thread_id,
                            request_id,
                            decision.as_str(),
                            err
                        );
                    }
                }
            }
        }
    }

    // Configured `beforeSubmitPrompt` hooks. Deliberately after the
    // approval-reply routing above: a bare "yes" answering a parked approval is
    // not a prompt the user is submitting to the model, and handing it to a
    // prompt hook would let a hook that blocks short messages strand a turn
    // waiting for an approval it can no longer receive.
    //
    // The message here is post-attachment-processing, so a hook sees extracted
    // text and placeholders rather than a multi-megabyte data URI on stdin.
    match crate::hooks::ops::prompt_submitted(
        tinyagents_runtime::command_hooks::context::TurnIdentity {
            conversation_id: Some(thread_id.clone()),
            ..Default::default()
        },
        &message,
        Vec::new(),
    )
    .await
    {
        crate::hooks::PromptVerdict::Submit { additional_context } => {
            if let Some(context) = additional_context {
                log::debug!(
                    "[web-channel] beforeSubmitPrompt hook added {} chars of context thread_id={}",
                    context.chars().count(),
                    thread_id
                );
                message = format!("{message}\n\n{context}");
            }
        }
        crate::hooks::PromptVerdict::Block(reason) => {
            log::info!(
                "[web-channel] prompt blocked by a configured hook thread_id={thread_id}: {reason}"
            );
            return Err(StartChatError::Other(reason));
        }
    }

    // A fresh accepted user request is the explicit boundary after Stop. Keep
    // the gate installed through validation and registry cancellation so a
    // child registering late cannot deliver into the stopped generation.
    crate::agent::orchestration::background_completions::resume_stopped_thread(ctx, &thread_id);

    let map_key = key_for(ctx, &thread_id);

    let parsed_mode = match queue_mode.as_deref() {
        Some("steer") => QueueMode::Steer,
        Some("followup") => QueueMode::Followup,
        Some("collect") => QueueMode::Collect,
        Some("parallel") => QueueMode::Parallel,
        _ => QueueMode::Interrupt,
    };

    // Parallel mode: spawn an independent forked turn that runs alongside any
    // in-flight turn for this thread. It does not touch IN_FLIGHT (no
    // interrupt/steer/queue) — it lives in its own request-keyed lane.
    if matches!(parsed_mode, QueueMode::Parallel) {
        log::info!(
            "[web-channel] starting PARALLEL forked turn thread_id={} request_id={}",
            thread_id,
            request_id
        );
        spawn_parallel_turn(
            ctx,
            &client_id,
            &thread_id,
            request_id.clone(),
            &message,
            model_override,
            temperature,
            locale,
            metadata,
        )
        .await;
        return Ok(request_id);
    }

    // Non-interrupt modes: push into the running turn's queue and return.
    if !matches!(parsed_mode, QueueMode::Interrupt) {
        let in_flight = in_flight(ctx).lock_owned().await;
        if let Some(existing) = in_flight.get(&map_key) {
            let item_id = uuid::Uuid::new_v4().to_string();
            let text_preview = crate::agent::queued_turn::text_preview(&message);
            let queued_msg = crate::agent::queued_turn::QueuedTurn {
                id: item_id.clone(),
                text: message.clone(),
                client_id: client_id.clone(),
                thread_id: thread_id.clone(),
                queued_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                model_override: model_override.clone(),
                temperature,
                locale: locale.clone(),
            };
            let lane = parsed_mode
                .queue_lane()
                .expect("only queueable modes reach an active run queue");
            existing.run_queue.push(lane, queued_msg).await;
            let status = existing.run_queue.status().await;
            log::info!(
                "[web-channel] queued {} message thread_id={} request_id={} queue_depth={} item_id={item_id}",
                parsed_mode,
                thread_id,
                request_id,
                status.total
            );
            crate::core::bus::BUS.publish(DomainEvent::RunQueueMessageQueued {
                thread_id: thread_id.clone(),
                mode: parsed_mode.to_string(),
                queue_depth: status.total,
                item_id: Some(item_id),
                text_preview: Some(text_preview),
            });
            return Ok(json!({
                "queued": true,
                "queue_mode": parsed_mode.to_string(),
                "client_id": client_id,
                "thread_id": thread_id,
                "request_id": request_id,
                "queue_depth": status.total,
            })
            .to_string());
        }
        log::info!(
            "[web-channel] no in-flight turn for {} mode thread_id={} — starting fresh",
            parsed_mode,
            thread_id
        );
    }

    {
        let mut in_flight = in_flight(ctx).lock_owned().await;

        if let Some(existing) = in_flight.remove(&map_key) {
            let cancelled_id = cancel_in_flight_gracefully(existing);
            log::info!(
                "[web-channel] interrupted in-flight turn thread_id={} cancelled_request_id={}",
                thread_id,
                cancelled_id
            );
            crate::core::bus::BUS.publish(DomainEvent::RunQueueInterrupted {
                thread_id: thread_id.clone(),
                cancelled_request_id: cancelled_id.clone(),
                item_id: Some(request_id.clone()),
                text_preview: Some(crate::agent::queued_turn::text_preview(&message)),
            });
            publish_web_channel_event_in(
                ctx,
                WebChannelEvent {
                    event: "chat_error".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    request_id: cancelled_id.clone(),
                    message: Some("Cancelled by newer request".to_string()),
                    error_type: Some("cancelled".to_string()),
                    ..Default::default()
                },
            );
            // See channel_ops::cancel_chat_inner — `chat_cancelled` is the
            // structured successor to `chat_error{error_type:"cancelled"}`,
            // kept alongside it for one release.
            publish_web_channel_event_in(
                ctx,
                WebChannelEvent {
                    event: "chat_cancelled".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    request_id: cancelled_id,
                    cancel_reason: Some("superseded".to_string()),
                    superseded_by: Some(request_id.clone()),
                    ..Default::default()
                },
            );
        }
    }

    let turn_run_queue = Arc::new(RunQueue::new());
    let turn_run_queue_task = turn_run_queue.clone();

    let client_id_task = client_id.clone();
    let thread_id_task = thread_id.clone();
    let request_id_task = request_id.clone();
    let map_key_task = map_key.clone();

    // Cooperative cancellation for this turn. The token lives in the
    // `InFlightEntry`; interrupt / cancel paths cancel it to tear the turn
    // future down gracefully at the next await point.
    let cancel_token = CancellationToken::new();
    let task_cancel_token = cancel_token.clone();

    let user_message = message.clone();
    let ctx_task = ctx.clone();
    let handle = tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
        async move {
            let approval_ctx = crate::security::approval::ApprovalChatContext {
                thread_id: thread_id_task.clone(),
                client_id: client_id_task.clone(),
                request_id: Some(request_id_task.clone()),
            };
            let origin = crate::agent::turn_origin::AgentTurnOrigin::WebChat {
                thread_id: thread_id_task.clone(),
                client_id: client_id_task.clone(),
                request_id: Some(request_id_task.clone()),
            };
            // `None` => the turn was cancelled cooperatively before producing a
            // result; the interrupting/cancelling side already emitted the
            // user-facing `chat_error`, so we just unwind quietly here.
            let result = run_turn_under_cancel_and_deadline(
                &ctx_task,
                task_cancel_token,
                origin.clone(),
                approval_ctx,
                |host_env, activity, approval| {
                    run_chat_task(
                        &ctx_task,
                        &client_id_task,
                        &thread_id_task,
                        &request_id_task,
                        &user_message,
                        model_override,
                        temperature,
                        locale,
                        turn_run_queue_task,
                        metadata,
                        origin.clone(),
                        /* fork */ false,
                        host_env,
                        activity,
                        approval,
                    )
                },
            )
            .await;

            let result = match result {
                Some(res) => res,
                None => {
                    log::info!(
                        "[web-channel] turn cancelled cooperatively client_id={} thread_id={} request_id={}",
                        client_id_task,
                        thread_id_task,
                        request_id_task
                    );
                    // Release any in-flight slot we still own and stop. The
                    // `request_id` guard below prevents clobbering a newer turn that
                    // replaced us on the interrupt path.
                    let mut in_flight = in_flight(&ctx_task).lock_owned().await;
                    if let Some(current) = in_flight.get(&map_key_task) {
                        if current.request_id == request_id_task {
                            in_flight.remove(&map_key_task);
                        }
                    }
                    return;
                }
            };

            // A terminal socket event is also the UI's permission to submit a
            // replacement turn. Defer both terminals until after the in-flight
            // slot below has been removed: publishing either while the slot is
            // still present lets the next message interrupt this finished run,
            // which then gets a spurious "cancelled" chat_error (and, after a
            // failure, lets an immediate retry inherit its failed model state).
            // The reply is stored now, before the slot is released (#6034).
            let mut deferred_done = None;
            let deferred_error = match result {
                Ok(chat_result) => {
                    crate::web_chat::presentation::persist_reply(
                        &ctx_task,
                        &thread_id_task,
                        &request_id_task,
                        &chat_result.full_response,
                        &chat_result.citations,
                        Some(chat_result.workspace_dir.as_path()),
                    )
                    .await;
                    deferred_done = Some(chat_result);
                    None
                }
                Err(err) => {
                    log::warn!(
                        "[web-channel] run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                        client_id_task,
                        thread_id_task,
                        request_id_task,
                        err
                    );
                    let detailed = format!(
                        "run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                        client_id_task, thread_id_task, request_id_task, err
                    );
                    let classified = classify_inference_error(&err);
                    let classified_type = classified.error_type;
                    let classified_type_string = classified_type.to_string();
                    if let Some(reason) = sentry_suppression_reason(&detailed) {
                        log::info!(
                            target: "web_channel",
                            "[web_channel.run_chat_task] suppressed Sentry emission for {} \
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
                            "run_chat_task",
                            &[
                                ("channel", "web"),
                                ("error_type", classified_type),
                                ("thread_id", thread_id_task.as_str()),
                                ("request_id", request_id_task.as_str()),
                                // Names which ceiling fired for the harness
                                // timeouts this arm now reports (#5804); "none"
                                // for every other error type.
                                ("timeout_bound", timeout_bound_tag(&detailed)),
                            ],
                        );
                    }
                    Some(WebChannelEvent {
                        event: "chat_error".to_string(),
                        client_id: client_id_task.clone(),
                        thread_id: thread_id_task.clone(),
                        request_id: request_id_task.clone(),
                        message: Some(classified.message),
                        error_type: Some(classified_type_string),
                        error_source: Some(classified.source.to_string()),
                        error_retryable: Some(classified.retryable),
                        error_retry_after_ms: classified.retry_after_ms,
                        error_provider: classified.provider,
                        error_fallback_available: classified.fallback_available,
                        copy_key: Some(classified.copy_key.to_string()),
                        copy_params: classified.copy_params,
                        ..Default::default()
                    })
                }
            };

            // Drain followup messages queued during this turn.
            let followups = {
                let mut in_flight = in_flight(&ctx_task).lock_owned().await;
                let followups = if let Some(current) = in_flight.get(&map_key_task) {
                    if current.request_id == request_id_task {
                        let fups = current.run_queue.drain(QueueLane::Followup).await;
                        in_flight.remove(&map_key_task);
                        fups
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                followups
            };
            // Announce the failed turn before starting its queued successor.
            // Socket subscribers then observe the old terminal event before
            // any inference_start from the follow-up, even if it starts at
            // once on another task.
            if let Some(chat_result) = deferred_done {
                crate::web_chat::presentation::announce_reply(
                    &ctx_task,
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &chat_result.full_response,
                    &user_message,
                    &chat_result.citations,
                    chat_result.usage.as_ref(),
                    chat_result.timing,
                    // The main single-user turn is the only surface with a
                    // human waiting on a next-message suggestion (C5).
                    true,
                );
            }
            if let Some(event) = deferred_error {
                publish_web_channel_event_in(&ctx_task, event);
            }
            if !followups.is_empty() {
                log::info!(
                    "[web-channel] dispatching {} followup(s) thread_id={}",
                    followups.len(),
                    thread_id_task
                );
                // `followups` can carry more than one drained item; the event's
                // `item_id`/`text_preview` describe the first one so the UI has
                // something concrete to show even when several dispatch at once.
                let first_followup = followups.first();
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::RunQueueFollowupDispatched {
                        thread_id: thread_id_task.clone(),
                        followup_count: followups.len(),
                        item_id: first_followup.map(|f| f.id.clone()),
                        text_preview: first_followup
                            .map(|f| crate::agent::queued_turn::text_preview(&f.text)),
                    },
                );
                dispatch_followups(&ctx_task, followups);
            }
        },
    ));

    {
        let mut in_flight = in_flight(ctx).lock_owned().await;
        in_flight.insert(
            map_key,
            InFlightEntry {
                request_id: request_id.clone(),
                client_id: client_id.clone(),
                handle,
                run_queue: turn_run_queue,
                cancel_token,
            },
        );
    }

    Ok(request_id)
}

#[cfg(test)]
#[path = "start_chat_tests.rs"]
mod tests;
