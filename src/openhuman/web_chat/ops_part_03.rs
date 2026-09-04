
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
        let approval_ctx = crate::openhuman::security::approval::ApprovalChatContext {
            thread_id: thread_id_task.clone(),
            client_id: client_id_task.clone(),
        };
        let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::WebChat {
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
                profile_id,
                locale,
                run_queue,
                metadata,
                /* fork */ true,
            ),
        )
        .await;

        match result {
            Some(Ok(chat_result)) => {
                crate::openhuman::web_chat::presentation::deliver_response(
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

/// Whether a cancel request should tear down the turn currently in flight for a
/// thread.
///
/// `requested` is the `request_id` the caller is cancelling; `None` means an
/// unscoped stop ("cancel whatever is running", e.g. a Stop button or a session
/// teardown). `in_flight` is the `request_id` currently registered for the
/// thread.
///
/// A *scoped* cancel matches only its own request. This is the fix for #4760: a
/// client that times out on request A and then sends request B — which
/// supersedes A on the same thread — must not have A's late-arriving cancel tear
/// down B. Scoping the cancel to A makes it a no-op once B is in flight, so the
/// newer turn survives instead of being killed at t=0.
pub fn cancel_should_target(requested: Option<&str>, in_flight: &str) -> bool {
    match requested {
        Some(rid) => rid == in_flight,
        None => true,
    }
}

/// Cancel a single parallel (forked) turn identified by `request_id`, but only
/// when it belongs to `thread_id`. Returns the cancelled id (as a one-element
/// vec, mirroring [`cancel_parallel_turns_for_thread`]) or empty when no such
/// parallel turn exists. Request-scoped cancel path (#4760).
async fn cancel_parallel_turn_by_request_id(thread_id: &str, request_id: &str) -> Vec<String> {
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

/// Cancel whatever turn is currently running on a thread (unscoped stop).
///
/// Back-compat entry point (Stop button / session teardown). For a cancel that
/// must only affect a specific turn — so a stale cancel can't kill a newer turn
/// on the same thread — use [`cancel_chat_scoped`] with the target `request_id`
/// (#4760).
pub async fn cancel_chat(client_id: &str, thread_id: &str) -> Result<Option<String>, String> {
    cancel_chat_scoped(client_id, thread_id, None).await
}

/// Cancel the in-flight turn(s) for a thread.
///
/// When `request_id` is `Some`, the cancel is **scoped**: it only tears down the
/// primary turn if that exact request is still running (and only the matching
/// parallel turn), so a stale cancel for a superseded request can't kill the
/// newer turn that replaced it (#4760). When `request_id` is `None`, it stops
/// whatever is running on the thread (primary + every parallel) — the "stop
/// everything" behaviour used by session teardown / a Stop button.
pub async fn cancel_chat_scoped(
    client_id: &str,
    thread_id: &str,
    request_id: Option<&str>,
) -> Result<Option<String>, String> {
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
        // #4760: only tear down the primary turn when the cancel is unscoped OR
        // targets exactly the request that is running. A stale cancel for an
        // already-superseded request must be a no-op so the newer turn lives.
        let should_cancel_primary = in_flight
            .get(&map_key)
            .map(|entry| cancel_should_target(request_id, &entry.request_id))
            .unwrap_or(false);
        if should_cancel_primary {
            if let Some(existing) = in_flight.remove(&map_key) {
                removed_request_id = Some(cancel_in_flight_gracefully(existing));
            }
        } else if let Some(rid) = request_id {
            log::info!(
                "[web-channel] ignoring stale cancel request_id={} for thread_id={} — current in-flight is {:?}; newer turn preserved",
                rid,
                thread_id,
                in_flight.get(&map_key).map(|e| e.request_id.as_str())
            );
        }
    }

    // Also tear down concurrent parallel (forked) turns. A scoped cancel targets
    // only the named parallel turn (if it is one); an unscoped cancel/stop
    // covers every parallel turn on the thread, not just the primary one.
    let cancelled_parallel = match request_id {
        Some(rid) => cancel_parallel_turn_by_request_id(thread_id, rid).await,
        None => cancel_parallel_turns_for_thread(thread_id).await,
    };

    // #4760: a scoped cancel that matched only a parallel (forked) turn — not the
    // primary — still genuinely tore a turn down and emitted its cancelled event.
    // Surface that id so `channel_web_cancel` reports `cancelled: true` with the
    // right request_id instead of misreporting a no-op just because the primary
    // turn wasn't the one cancelled.
    let cancelled_any = removed_request_id
        .clone()
        .or_else(|| cancelled_parallel.first().cloned());

    // Emit a cancelled chat_error for each cancelled turn (primary + parallels)
    // so every interleaved branch's UI is resolved.
    for request_id in removed_request_id.into_iter().chain(cancelled_parallel) {
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

    Ok(cancelled_any)
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
    request_id: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let cancelled_request_id = cancel_chat_scoped(client_id, thread_id, request_id).await?;

    // No web-channel turn matched. Fall through to the task-dispatcher registry,
    // which holds autonomous runs that are NOT web-channel turns (so they never
    // appear in IN_FLIGHT and can only be reached here). The fallback is itself
    // request-scoped: a scoped cancel aborts the run only when its run_id
    // matches, so a stale cancel for a superseded request can't tear down a newer
    // run on the thread (#4760); an unscoped stop aborts whatever run is running.
    let cancelled = if cancelled_request_id.is_some() {
        true
    } else {
        crate::openhuman::agent::task_dispatcher::cancel_session_scoped(
            thread_id.trim(),
            request_id,
        )
        .await
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
