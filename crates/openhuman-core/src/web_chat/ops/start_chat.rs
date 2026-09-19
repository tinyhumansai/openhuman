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
use crate::core::socketio::WebChannelEvent;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};

use super::super::event_bus::publish_web_channel_event;
use super::super::run_task::run_chat_task;
use super::super::types::{ChatRequestMetadata, InFlightEntry, QueueMode};
use super::super::web_errors::classify_inference_error;
use super::parallel_turn::spawn_parallel_turn;
use super::state::{cancel_in_flight_gracefully, key_for, IN_FLIGHT};
use super::turn_guards::{
    run_turn_under_cancel_and_deadline, sentry_suppression_reason, timeout_bound_tag,
};

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

pub async fn start_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    queue_mode: Option<String>,
    metadata: ChatRequestMetadata,
) -> Result<String, String> {
    let client_id = client_id.trim().to_string();
    let thread_id = thread_id.trim().to_string();
    let message = message.trim().to_string();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    if message.is_empty() {
        return Err("message is required".to_string());
    }

    // [pdf/image-attach fix] Process attachments at ingress, BEFORE the message is
    // injection-scanned, persisted to history/JSONL, or auto-saved to the memory
    // store. Otherwise a multi-MB base64 data URI floods every upstream stage
    // (N-chunk embed → Voyage 400, cross-thread index) and stalls the turn.
    //   [FILE:data:…]  → [FILE-EXTRACTED]text (or [FILE-ATTACHED] placeholder)
    //   [IMAGE:data:…] → [Image: … #att:<id>] placeholder + out-of-band stash
    // Images are rehydrated to a data URI at provider dispatch for vision-capable
    // models only.
    let mut message = if message.contains("[FILE:") || message.contains("[IMAGE:") {
        let before_chars = message.chars().count();
        log::debug!(
            "[web-channel][ingress] preprocessing attachment markers thread_id={} client_id={} chars={}",
            thread_id,
            client_id,
            before_chars
        );
        // Fail CLOSED on a config-load error: process with default limits rather
        // than passing the raw `[FILE:data:…]`/`[IMAGE:data:…]` blob through —
        // otherwise the injection scan, history/JSONL persistence, and memory
        // autosave all see the multi-MB data URI again, reopening the flood path.
        let (file_cfg, image_cfg) = match crate::config::rpc::load_config_with_timeout().await {
            Ok(cfg) => {
                log::debug!(
                    "[web-channel][ingress] using configured multimodal limits thread_id={}",
                    thread_id
                );
                (cfg.multimodal_files, cfg.multimodal)
            }
            Err(err) => {
                log::warn!(
                    "[web-channel][ingress] config load failed; using default limits (fail-closed) thread_id={} err={err}",
                    thread_id
                );
                (
                    crate::config::MultimodalFileConfig::default(),
                    crate::config::MultimodalConfig::default(),
                )
            }
        };
        let extracted =
            crate::agent::multimodal::inline_file_attachments(&message, &file_cfg).await;
        let processed =
            crate::agent::multimodal::stash_image_attachments(&extracted, &image_cfg).await;
        log::debug!(
            "[web-channel][ingress] attachment preprocessing complete thread_id={} before_chars={} after_chars={}",
            thread_id,
            before_chars,
            processed.chars().count()
        );
        processed
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
        return Err(prompt_guard_user_message(prompt_decision.action).to_string());
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
        crate::hooks::context::TurnIdentity {
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
            return Err(reason);
        }
    }

    let map_key = key_for(&thread_id);

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
        let in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.get(&map_key) {
            let queued_msg = crate::agent::queued_turn::QueuedTurn {
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
                "[web-channel] queued {} message thread_id={} request_id={} queue_depth={}",
                parsed_mode,
                thread_id,
                request_id,
                status.total
            );
            crate::core::bus::BUS.publish(DomainEvent::RunQueueMessageQueued {
                thread_id: thread_id.clone(),
                mode: parsed_mode.to_string(),
                queue_depth: status.total,
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
        let mut in_flight = IN_FLIGHT.lock().await;

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
            });
            publish_web_channel_event(WebChannelEvent {
                event: "chat_error".to_string(),
                client_id: client_id.clone(),
                thread_id: thread_id.clone(),
                request_id: cancelled_id,
                message: Some("Cancelled by newer request".to_string()),
                error_type: Some("cancelled".to_string()),
                ..Default::default()
            });
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
            // `None` => the turn was cancelled cooperatively before producing a
            // result; the interrupting/cancelling side already emitted the
            // user-facing `chat_error`, so we just unwind quietly here.
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
                    turn_run_queue_task,
                    metadata,
                    /* fork */ false,
                ),
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
                    let mut in_flight = IN_FLIGHT.lock().await;
                    if let Some(current) = in_flight.get(&map_key_task) {
                        if current.request_id == request_id_task {
                            in_flight.remove(&map_key_task);
                        }
                    }
                    return;
                }
            };

            match result {
                Ok(chat_result) => {
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
                    publish_web_channel_event(WebChannelEvent {
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
                        ..Default::default()
                    });
                }
            }

            // Drain followup messages queued during this turn.
            let followups = {
                let mut in_flight = IN_FLIGHT.lock().await;
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
            if !followups.is_empty() {
                log::info!(
                    "[web-channel] dispatching {} followup(s) thread_id={}",
                    followups.len(),
                    thread_id_task
                );
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::RunQueueFollowupDispatched {
                        thread_id: thread_id_task.clone(),
                        followup_count: followups.len(),
                    },
                );
                dispatch_followups(followups);
            }
        },
    ));

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        in_flight.insert(
            map_key,
            InFlightEntry {
                request_id: request_id.clone(),
                handle,
                run_queue: turn_run_queue,
                cancel_token,
            },
        );
    }

    Ok(request_id)
}

fn dispatch_followups(followups: Vec<crate::agent::queued_turn::QueuedTurn>) {
    for fup in followups {
        tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
            async move {
                if let Err(err) = start_chat(
                    &fup.client_id,
                    &fup.thread_id,
                    &fup.text,
                    fup.model_override,
                    fup.temperature,
                    fup.locale,
                    Some("followup".to_string()),
                    ChatRequestMetadata::default(),
                )
                .await
                {
                    log::warn!(
                        "[web-channel] failed to dispatch followup thread_id={} err={}",
                        fup.thread_id,
                        err
                    );
                }
            },
        ));
    }
}
