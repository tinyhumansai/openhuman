use std::collections::HashMap;
use std::time::Duration;

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::core::event_bus::DomainEvent;
use crate::core::socketio::WebChannelEvent;
use crate::openhuman::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::rpc::RpcOutcome;

use super::event_bus::publish_web_channel_event;
use super::run_task::run_chat_task;
use super::types::{ChatRequestMetadata, InFlightEntry, ParallelEntry, SessionEntry};
use super::web_errors::classify_inference_error;

pub(crate) static THREAD_SESSIONS: Lazy<Mutex<HashMap<String, SessionEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub(super) static IN_FLIGHT: Lazy<Mutex<HashMap<String, InFlightEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Parallel (forked) turns, keyed by `request_id`. A separate lane from
/// `IN_FLIGHT` (which holds one primary, interrupt-able turn per thread) so any
/// number of concurrent `QueueMode::Parallel` turns can run on the same thread
/// without touching interrupt/steer/queue semantics. See `QueueMode::Parallel`.
pub(super) static PARALLEL_IN_FLIGHT: Lazy<Mutex<HashMap<String, ParallelEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(any(test, debug_assertions))]
pub(super) static TEST_FORCED_RUN_CHAT_TASK_ERROR: Lazy<Mutex<Option<String>>> =
    Lazy::new(|| Mutex::new(None));

/// Test hook handles: when set, `run_chat_task` parks on a long sleep instead
/// of doing real work, keeping the turn in-flight so concurrency / cancellation
/// can be observed. `started` is flipped once the turn has actually parked (so
/// a test can cancel only after the turn future is live), and a `Drop` guard
/// inside the parked future flips `dropped`, proving cooperative cancellation
/// tears the turn future down (vs. a hard `abort()` that never runs the Drop).
#[cfg(any(test, debug_assertions))]
#[derive(Clone)]
pub struct TestRunChatTaskBlock {
    pub started: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(any(test, debug_assertions))]
pub(super) static TEST_RUN_CHAT_TASK_BLOCK: Lazy<Mutex<Option<TestRunChatTaskBlock>>> =
    Lazy::new(|| Mutex::new(None));

/// Cooperatively cancel an in-flight turn, with a hard `abort()` backstop.
///
/// Cancelling the token makes the turn's `tokio::select!` arm fire, dropping
/// the turn future at its next await point (cancelling the in-flight LLM
/// request and releasing locks cleanly). The detached backstop hard-aborts the
/// task only if it has not finished unwinding within a short grace period, so a
/// wedged turn can never leak. Returns the cancelled turn's request id.
fn cancel_in_flight_gracefully(entry: InFlightEntry) -> String {
    let request_id = entry.request_id.clone();
    entry.cancel_token.cancel();
    let mut handle = entry.handle;
    tokio::spawn(async move {
        tokio::select! {
            _ = &mut handle => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                log::warn!(
                    "[web-channel] cooperative cancel did not finish within grace period — hard-aborting backstop"
                );
                handle.abort();
            }
        }
    });
    request_id
}

pub(crate) fn key_for(thread_id: &str) -> String {
    thread_id.to_string()
}

pub(crate) fn event_session_id_for(client_id: &str, thread_id: &str) -> String {
    json!({
        "client_id": client_id,
        "thread_id": thread_id,
    })
    .to_string()
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

#[cfg(any(test, debug_assertions))]
pub async fn set_test_forced_run_chat_task_error(message: Option<&str>) {
    let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
    *slot = message.map(str::to_string);
}

/// Test hook: when `block` is `Some`, the next `run_chat_task` invocations park
/// on a long sleep (staying in-flight), flip `started` once parked, and flip
/// `dropped` when their future is torn down. Pass `None` to clear.
#[cfg(any(test, debug_assertions))]
pub async fn set_test_run_chat_task_block(block: Option<TestRunChatTaskBlock>) {
    let mut slot = TEST_RUN_CHAT_TASK_BLOCK.lock().await;
    *slot = block;
}

pub async fn start_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
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
    // models only (engine::core).
    let message = if message.contains("[FILE:") || message.contains("[IMAGE:") {
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
        let (file_cfg, image_cfg) = match crate::openhuman::config::rpc::load_config_with_timeout()
            .await
        {
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
                    crate::openhuman::config::MultimodalFileConfig::default(),
                    crate::openhuman::config::MultimodalConfig::default(),
                )
            }
        };
        let extracted =
            crate::openhuman::agent::multimodal::inline_file_attachments(&message, &file_cfg).await;
        let processed =
            crate::openhuman::agent::multimodal::stash_image_attachments(&extracted, &image_cfg)
                .await;
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
            source: "channels.providers.web.start_chat",
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
    if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
        if let Some(request_id) = gate.pending_for_thread(&thread_id) {
            if let Some(decision) = crate::openhuman::approval::parse_approval_reply(&message) {
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

    let map_key = key_for(&thread_id);

    let parsed_mode = match queue_mode.as_deref() {
        Some("steer") => crate::openhuman::agent::harness::run_queue::QueueMode::Steer,
        Some("followup") => crate::openhuman::agent::harness::run_queue::QueueMode::Followup,
        Some("collect") => crate::openhuman::agent::harness::run_queue::QueueMode::Collect,
        Some("parallel") => crate::openhuman::agent::harness::run_queue::QueueMode::Parallel,
        _ => crate::openhuman::agent::harness::run_queue::QueueMode::Interrupt,
    };

    // Parallel mode: spawn an independent forked turn that runs alongside any
    // in-flight turn for this thread. It does not touch IN_FLIGHT (no
    // interrupt/steer/queue) — it lives in its own request-keyed lane.
    if matches!(
        parsed_mode,
        crate::openhuman::agent::harness::run_queue::QueueMode::Parallel
    ) {
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
            profile_id,
            locale,
            metadata,
        )
        .await;
        return Ok(request_id);
    }

    // Non-interrupt modes: push into the running turn's queue and return.
    if !matches!(
        parsed_mode,
        crate::openhuman::agent::harness::run_queue::QueueMode::Interrupt
    ) {
        let in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.get(&map_key) {
            let queued_msg = crate::openhuman::agent::harness::run_queue::QueuedMessage {
                text: message.clone(),
                mode: parsed_mode,
                client_id: client_id.clone(),
                thread_id: thread_id.clone(),
                queued_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                model_override: model_override.clone(),
                temperature,
                profile_id: profile_id.clone(),
                locale: locale.clone(),
            };
            existing.run_queue.push(queued_msg).await;
            let status = existing.run_queue.status().await;
            log::info!(
                "[web-channel] queued {} message thread_id={} request_id={} queue_depth={}",
                parsed_mode,
                thread_id,
                request_id,
                status.total
            );
            crate::core::event_bus::publish_global(DomainEvent::RunQueueMessageQueued {
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
            crate::core::event_bus::publish_global(DomainEvent::RunQueueInterrupted {
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

    let turn_run_queue = crate::openhuman::agent::harness::run_queue::RunQueue::new();
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
    let handle = tokio::spawn(async move {
        let approval_ctx = crate::openhuman::approval::ApprovalChatContext {
            thread_id: thread_id_task.clone(),
            client_id: client_id_task.clone(),
        };
        let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::WebChat {
            thread_id: thread_id_task.clone(),
            client_id: client_id_task.clone(),
        };
        // `None` => the turn was cancelled cooperatively before producing a
        // result; the interrupting/cancelling side already emitted the
        // user-facing `chat_error`, so we just unwind quietly here.
        let result = tokio::select! {
            biased;
            _ = task_cancel_token.cancelled() => None,
            res = crate::openhuman::agent::turn_origin::with_origin(
                origin,
                crate::openhuman::approval::APPROVAL_CHAT_CONTEXT.scope(
                    approval_ctx,
                    run_chat_task(
                        &client_id_task,
                        &thread_id_task,
                        &request_id_task,
                        &user_message,
                        model_override,
                        temperature,
                        profile_id,
                        locale,
                        turn_run_queue_task,
                        metadata,
                        /* fork */ false,
                    ),
                ),
            ) => Some(res),
        };

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
                crate::openhuman::channels::providers::presentation::deliver_response(
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &chat_result.full_response,
                    &user_message,
                    &chat_result.citations,
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
                if crate::openhuman::agent::error::is_max_iterations_error(&detailed) {
                    log::info!(
                        target: "web_channel",
                        "[web_channel.run_chat_task] suppressed Sentry emission for max-iteration \
                         cap client_id={} thread_id={} request_id={} error_type={} message={}",
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
                    let fups = current.run_queue.drain_followups().await;
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
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::RunQueueFollowupDispatched {
                    thread_id: thread_id_task.clone(),
                    followup_count: followups.len(),
                },
            );
            dispatch_followups(followups);
        }
    });

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

fn dispatch_followups(followups: Vec<crate::openhuman::agent::harness::run_queue::QueuedMessage>) {
    for fup in followups {
        tokio::spawn(async move {
            if let Err(err) = start_chat(
                &fup.client_id,
                &fup.thread_id,
                &fup.text,
                fup.model_override,
                fup.temperature,
                fup.profile_id,
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
        });
    }
}

/// Spawn an independent, forked (`QueueMode::Parallel`) turn. It snapshots the
/// thread's history-at-start (inside `run_chat_task` with `fork = true`), runs
/// concurrently with any other turn on the thread, and on completion delivers
/// its response (append-only) and removes itself from `PARALLEL_IN_FLIGHT`.
/// Emits the same per-`request_id` stream events as a primary turn, so the UI
/// can render it as an interleaved branch.
#[allow(clippy::too_many_arguments)]
async fn spawn_parallel_turn(
    client_id: &str,
    thread_id: &str,
    request_id: String,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
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
    let run_queue = crate::openhuman::agent::harness::run_queue::RunQueue::new();

    let handle = tokio::spawn(async move {
        let approval_ctx = crate::openhuman::approval::ApprovalChatContext {
            thread_id: thread_id_task.clone(),
            client_id: client_id_task.clone(),
        };
        let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::WebChat {
            thread_id: thread_id_task.clone(),
            client_id: client_id_task.clone(),
        };
        let result = tokio::select! {
            biased;
            _ = task_cancel_token.cancelled() => None,
            res = crate::openhuman::agent::turn_origin::with_origin(
                origin,
                crate::openhuman::approval::APPROVAL_CHAT_CONTEXT.scope(
                    approval_ctx,
                    run_chat_task(
                        &client_id_task,
                        &thread_id_task,
                        &request_id_task,
                        &user_message,
                        model_override,
                        temperature,
                        profile_id,
                        locale,
                        run_queue,
                        metadata,
                        /* fork */ true,
                    ),
                ),
            ) => Some(res),
        };

        match result {
            Some(Ok(chat_result)) => {
                crate::openhuman::channels::providers::presentation::deliver_response(
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &chat_result.full_response,
                    &user_message,
                    &chat_result.citations,
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
                let classified = classify_inference_error(&err);
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
    });

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
async fn cancel_parallel_turns_for_thread(thread_id: &str) -> Vec<String> {
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

pub async fn invalidate_thread_sessions(thread_id: &str) {
    let mut sessions = THREAD_SESSIONS.lock().await;
    let keys_to_remove: Vec<String> = sessions
        .keys()
        .filter(|k| k.as_str() == thread_id || k.ends_with(&format!("::{thread_id}")))
        .cloned()
        .collect();
    for key in &keys_to_remove {
        sessions.remove(key);
    }
    if !keys_to_remove.is_empty() {
        log::debug!(
            "[web-channel] invalidated {} cached session(s) for thread_id={}",
            keys_to_remove.len(),
            thread_id
        );
    }
}

pub async fn in_flight_entries_for_test() -> Vec<(String, String)> {
    let guard = IN_FLIGHT.lock().await;
    guard
        .iter()
        .map(|(k, v)| (k.clone(), v.request_id.clone()))
        .collect()
}

/// Test accessor: `(request_id, thread_id)` for every in-flight parallel turn.
#[cfg(any(test, debug_assertions))]
pub async fn parallel_in_flight_entries_for_test() -> Vec<(String, String)> {
    let guard = PARALLEL_IN_FLIGHT.lock().await;
    guard
        .iter()
        .map(|(request_id, entry)| (request_id.clone(), entry.thread_id.clone()))
        .collect()
}

pub async fn cancel_chat(client_id: &str, thread_id: &str) -> Result<Option<String>, String> {
    let client_id = client_id.trim();
    let thread_id = thread_id.trim();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }

    let map_key = key_for(thread_id);
    let mut removed_request_id: Option<String> = None;

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.remove(&map_key) {
            removed_request_id = Some(cancel_in_flight_gracefully(existing));
        }
    }

    // Also tear down any concurrent parallel (forked) turns on the thread so a
    // cancel/stop covers every in-flight turn, not just the primary one.
    let cancelled_parallel = cancel_parallel_turns_for_thread(thread_id).await;

    // Emit a cancelled chat_error for each cancelled turn (primary + parallels)
    // so every interleaved branch's UI is resolved.
    for request_id in removed_request_id.iter().cloned().chain(cancelled_parallel) {
        publish_web_channel_event(WebChannelEvent {
            event: "chat_error".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id,
            message: Some("Cancelled".to_string()),
            error_type: Some("cancelled".to_string()),
            ..Default::default()
        });
    }

    Ok(removed_request_id)
}

pub async fn channel_web_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
    locale: Option<String>,
    queue_mode: Option<String>,
    metadata: ChatRequestMetadata,
) -> Result<RpcOutcome<Value>, String> {
    let result = start_chat(
        client_id,
        thread_id,
        message,
        model_override,
        temperature,
        profile_id,
        locale,
        queue_mode,
        metadata,
    )
    .await?;

    if let Ok(parsed) = serde_json::from_str::<Value>(&result) {
        return Ok(RpcOutcome::single_log(parsed, "web channel message queued"));
    }

    Ok(RpcOutcome::single_log(
        json!({
            "accepted": true,
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": result,
        }),
        "web channel request accepted",
    ))
}

pub async fn channel_web_queue_status(thread_id: &str) -> Result<RpcOutcome<Value>, String> {
    let map_key = key_for(thread_id);
    let in_flight = IN_FLIGHT.lock().await;
    if let Some(entry) = in_flight.get(&map_key) {
        let status = entry.run_queue.status().await;
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "active": true,
                "request_id": entry.request_id,
                "steers": status.steers,
                "followups": status.followups,
                "collects": status.collects,
                "total": status.total,
            }),
            "queue status retrieved",
        ))
    } else {
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "active": false,
                "steers": 0,
                "followups": 0,
                "collects": 0,
                "total": 0,
            }),
            "no active turn for thread",
        ))
    }
}

pub async fn channel_web_queue_clear(thread_id: &str) -> Result<RpcOutcome<Value>, String> {
    let map_key = key_for(thread_id);
    let in_flight = IN_FLIGHT.lock().await;
    if let Some(entry) = in_flight.get(&map_key) {
        let dropped = entry.run_queue.clear().await;
        log::info!(
            "[web-channel] cleared queue thread_id={} dropped={}",
            thread_id,
            dropped
        );
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "cleared": true,
                "dropped": dropped,
            }),
            "queue cleared",
        ))
    } else {
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "cleared": false,
                "dropped": 0,
            }),
            "no active turn for thread",
        ))
    }
}

pub async fn channel_web_cancel(
    client_id: &str,
    thread_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let cancelled_request_id = cancel_chat(client_id, thread_id).await?;

    let cancelled = if cancelled_request_id.is_some() {
        true
    } else {
        crate::openhuman::agent::task_dispatcher::cancel_session(thread_id.trim()).await
    };

    Ok(RpcOutcome::single_log(
        json!({
            "cancelled": cancelled,
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": cancelled_request_id,
        }),
        "web channel cancellation processed",
    ))
}
