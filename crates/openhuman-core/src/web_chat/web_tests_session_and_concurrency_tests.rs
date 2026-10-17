use super::*;

#[test]
fn fingerprint_provider_binding_change_forces_rebuild() {
    // The whole point of adding provider_binding to the fingerprint:
    // changing the workload routing in Connections → API keys → LLM mid-thread
    // must invalidate the cached agent so the next turn rebuilds with
    // the new provider.
    let warm = fp(None, None, "orchestrator", "cloud");
    let after_settings_change = fp(None, None, "orchestrator", "anthropic:claude-sonnet-4-6");
    assert_ne!(
        warm, after_settings_change,
        "provider binding change must produce a different fingerprint (cache miss → rebuild)"
    );
}

#[test]
fn fingerprint_provider_binding_variants_differ() {
    let unset = fp(None, None, "orchestrator", "openhuman");
    let set = fp(None, None, "orchestrator", "cloud");
    assert_ne!(unset, set);
}

#[test]
fn provider_role_override_routes_hint_workloads() {
    assert_eq!(
        provider_role_for_model_override(Some("hint:agentic")),
        "agentic"
    );
    assert_eq!(
        provider_role_for_model_override(Some("agentic-v1")),
        "agentic"
    );
    assert_eq!(
        provider_role_for_model_override(Some("hint:coding")),
        "coding"
    );
    assert_eq!(
        provider_role_for_model_override(Some("summarization-v1")),
        "summarization"
    );
    assert_eq!(
        provider_role_for_model_override(Some("hint:reasoning")),
        "reasoning"
    );
    assert_eq!(
        provider_role_for_model_override(Some("reasoning-v1")),
        "reasoning"
    );
    assert_eq!(
        provider_role_for_model_override(Some("gpt-4.1-mini")),
        "chat"
    );
    assert_eq!(provider_role_for_model_override(None), "chat");
}

#[test]
fn fingerprint_target_agent_flip_forces_rebuild() {
    let orchestrator = fp(None, None, "orchestrator", "cloud");
    let profile_agent = fp(None, None, "integrations_agent", "cloud");
    assert_ne!(orchestrator, profile_agent);
}

#[test]
fn fingerprint_model_override_and_temperature_participate() {
    let base = fp(None, None, "orchestrator", "cloud");
    assert_ne!(
        base,
        fp(Some("gpt-4o"), None, "orchestrator", "cloud"),
        "per-message model_override must invalidate"
    );
    assert_ne!(
        base,
        fp(None, Some(0.9), "orchestrator", "cloud"),
        "per-message temperature must invalidate"
    );
}

#[test]
fn locale_reply_directive_returns_none_for_english() {
    assert!(locale_reply_directive("en").is_none());
    // Unrecognised tags fall through too — the agent's default is fine.
    assert!(locale_reply_directive("xx").is_none());
    assert!(locale_reply_directive("").is_none());
}

#[test]
fn locale_reply_directive_renders_known_locales() {
    let ar = locale_reply_directive("ar").expect("arabic directive expected");
    assert!(
        ar.contains("Arabic"),
        "directive must name the language: {ar}"
    );
    assert!(
        ar.contains("Respond in Arabic"),
        "directive must instruct the agent: {ar}"
    );
    let zh = locale_reply_directive("zh-CN").expect("zh-CN directive expected");
    assert!(zh.contains("Simplified Chinese"));
}

/// Every non-English locale the app offers (`app/src/lib/i18n/types.ts`) gets
/// a directive; de, ja, ko, pl and tr used to fall through to none.
#[test]
fn locale_reply_directive_covers_every_app_locale() {
    for locale in [
        "zh-CN", "hi", "es", "ar", "fr", "bn", "pt", "de", "ru", "id", "it", "ja", "ko", "pl", "tr",
    ] {
        assert!(
            locale_reply_directive(locale).is_some(),
            "no reply directive for app locale {locale}"
        );
    }
}

// ── PTT field additions (Task 1 of global-ptt plan) ─────────────────────────

#[test]
fn web_chat_schema_accepts_optional_ptt_fields() {
    // Locate the `chat` schema via the public accessor.
    let schema = schemas("chat");
    let names: std::collections::HashSet<&str> = schema.inputs.iter().map(|f| f.name).collect();
    assert!(
        names.contains("speak_reply"),
        "channel.web_chat schema must include optional speak_reply field"
    );
    assert!(
        names.contains("source"),
        "channel.web_chat schema must include optional source field"
    );
    assert!(
        names.contains("session_id"),
        "channel.web_chat schema must include optional session_id field"
    );
    // All three are optional.
    for field in &["speak_reply", "source", "session_id"] {
        let f = schema
            .inputs
            .iter()
            .find(|f| f.name == *field)
            .expect("field present");
        assert!(!f.required, "{field} must be optional");
    }
    // Type assertions: ensure each field has the correct wire type.
    let speak_reply = schema
        .inputs
        .iter()
        .find(|f| f.name == "speak_reply")
        .unwrap();
    assert_eq!(
        speak_reply.ty,
        TypeSchema::Option(Box::new(TypeSchema::Bool)),
        "speak_reply must be Option<bool>"
    );
    let source = schema.inputs.iter().find(|f| f.name == "source").unwrap();
    assert_eq!(
        source.ty,
        TypeSchema::Option(Box::new(TypeSchema::String)),
        "source must be Option<String>"
    );
    let session_id = schema
        .inputs
        .iter()
        .find(|f| f.name == "session_id")
        .unwrap();
    assert_eq!(
        session_id.ty,
        TypeSchema::Option(Box::new(TypeSchema::U64)),
        "session_id must be Option<u64>"
    );
}

#[test]
fn web_chat_params_deserialize_with_all_ptt_fields_omitted() {
    let json = serde_json::json!({
        "client_id": "c1",
        "thread_id": "t1",
        "message": "hello",
    });
    let parsed: WebChatParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.speak_reply, None);
    assert_eq!(parsed.source, None);
    assert_eq!(parsed.session_id, None);
}

#[test]
fn web_chat_params_deserialize_with_all_ptt_fields_present() {
    let json = serde_json::json!({
        "client_id": "c1",
        "thread_id": "t1",
        "message": "hello",
        "speak_reply": true,
        "source": "ptt",
        "session_id": 42_u64,
    });
    let parsed: WebChatParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.speak_reply, Some(true));
    assert_eq!(parsed.source.as_deref(), Some("ptt"));
    assert_eq!(parsed.session_id, Some(42));
}

/// Two turns on DISTINCT threads must be in-flight at the same time — the core
/// invariant behind cross-thread parallel inference.
#[tokio::test]
async fn start_chat_runs_distinct_threads_concurrently() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;

    let thread_a = "concurrent-thread-a";
    let thread_b = "concurrent-thread-b";

    start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "client-a",
        thread_a,
        "hello a",
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("thread A should start");
    start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "client-b",
        thread_b,
        "hello b",
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("thread B should start");

    // Both threads' turns must be parked in-flight simultaneously.
    let entries = wait_for_in_flight(|e| {
        let keys: Vec<&str> = e.iter().map(|(k, _)| k.as_str()).collect();
        keys.contains(&thread_a) && keys.contains(&thread_b)
    })
    .await;
    assert!(
        entries.iter().any(|(k, _)| k == thread_a) && entries.iter().any(|(k, _)| k == thread_b),
        "expected both threads in-flight concurrently, got {entries:?}"
    );

    // Cleanup: cancel both and clear the test hook.
    let _ = cancel_chat_scoped(
        &crate::core::runtime::DispatchCtx::ambient(),
        "client-a",
        thread_a,
        None,
    )
    .await;
    let _ = cancel_chat_scoped(
        &crate::core::runtime::DispatchCtx::ambient(),
        "client-b",
        thread_b,
        None,
    )
    .await;
    set_test_run_chat_task_block(None).await;
}

/// `cancel_chat` must cooperatively tear down the in-flight turn (drop its
/// future at the next await point) rather than leave it sleeping — proven by
/// the parked future's `Drop` guard firing well before its 30s sleep elapses.
#[tokio::test]
async fn cancel_chat_cooperatively_stops_in_flight_turn() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;

    let thread_id = "cancel-coop-thread";
    let request_id = start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "cancel-client",
        thread_id,
        "park me",
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("turn should start");

    // Wait until the turn future has actually parked (guard created) — only then
    // is a cooperative cancel meaningful.
    wait_for_flag(&block.started, "turn started").await;
    assert!(
        !block.dropped.load(Ordering::SeqCst),
        "turn should still be parked, not yet dropped"
    );

    let cancelled = cancel_chat_scoped(
        &crate::core::runtime::DispatchCtx::ambient(),
        "cancel-client",
        thread_id,
        None,
    )
    .await
    .expect("cancel_chat should succeed");
    assert_eq!(
        cancelled.as_deref(),
        Some(request_id.as_str()),
        "cancel_chat should report the cancelled request id"
    );

    // The in-flight entry is removed and the parked future is dropped promptly
    // (cooperative cancel), long before the 30s test sleep would elapse.
    wait_for_in_flight(|e| !e.iter().any(|(k, _)| k == thread_id)).await;
    wait_for_flag(&block.dropped, "turn future dropped by cooperative cancel").await;

    set_test_run_chat_task_block(None).await;
}

/// Issue #4746: a turn that never produces a terminal event on its own — a
/// wedged main agent stuck mid tool-call, or a delegated sub-agent that never
/// returns (modeled here by the parked test block) — must still end in a
/// terminal event, never an empty reply / an endless `inference_heartbeat`
/// stream until the socket dies. The web turn driver's wall-clock backstop fires
/// and tears the wedged future down cooperatively (its `Drop` guard flips well
/// before the 30s test sleep would elapse). The block parks before the turn
/// resolves its config, so there is no workspace to write a closing reply to:
/// the turn ends in the graceful, retryable `turn_timeout` chat_error.
#[tokio::test]
async fn wedged_turn_hits_wall_clock_backstop_and_emits_turn_timeout_chat_error() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    // Panic-safe teardown of the process-global env override: if any assertion
    // below unwinds, this guard still clears `OPENHUMAN_WEB_TURN_TIMEOUT_SECS` so
    // a 1s backstop can't leak into unrelated tests sharing this process.
    let _env_guard = crate::config::test_env::EnvVarGuard::locked_set_async(
        "OPENHUMAN_WEB_TURN_TIMEOUT_SECS",
        "1",
    )
    .await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;

    let mut rx = subscribe_web_channel_events();
    let request_id = start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "backstop-client",
        "backstop-thread",
        "park me until the wall-clock backstop fires",
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("turn should start");

    let recv = timeout(
        Duration::from_secs(20),
        terminal_event(&mut rx, &request_id),
    )
    .await
    .expect("a wedged turn must still emit a terminal event (issue #4746)");
    assert_turn_timeout(&recv);

    // The wedged future is torn down cooperatively when the backstop drops it
    // (the Drop guard flips), not left sleeping to its 30s test wall.
    wait_for_flag(&block.dropped, "wedged turn future dropped by backstop").await;

    set_test_run_chat_task_block(None).await;
    // `OPENHUMAN_WEB_TURN_TIMEOUT_SECS` is cleared by `_env_guard`'s Drop.
}

/// The idle guard ends a turn that emits no progress with nothing in flight,
/// well inside the wall-clock ceiling. The parked test block emits nothing and
/// parks before the turn resolves its config, like a turn wedged in session
/// checkout, so the turn ends in the graceful `turn_timeout` chat_error.
#[tokio::test]
async fn silent_turn_hits_idle_guard_and_emits_turn_timeout_chat_error() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let _env_guard =
        crate::config::test_env::EnvVarGuard::locked_set_async("OPENHUMAN_WEB_TURN_IDLE_SECS", "1")
            .await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;

    let mut rx = subscribe_web_channel_events();
    let request_id = start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "idle-client",
        "idle-thread",
        "park me until the idle guard fires",
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("turn should start");

    let recv = timeout(
        Duration::from_secs(20),
        terminal_event(&mut rx, &request_id),
    )
    .await
    .expect("a silent turn must end in a terminal event");
    assert_turn_timeout(&recv);
    wait_for_flag(&block.dropped, "silent turn future dropped by idle guard").await;

    set_test_run_chat_task_block(None).await;
}

/// A `parallel`-mode turn runs CONCURRENTLY with the primary turn on the SAME
/// thread (it does not interrupt it), and a thread-level cancel tears down both.
#[tokio::test]
async fn parallel_turn_runs_concurrently_with_primary_on_same_thread() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;

    let thread_id = "parallel-same-thread";

    // Primary turn (default interrupt mode) parks in IN_FLIGHT.
    start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "pp-client",
        thread_id,
        "primary",
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("primary turn should start");
    wait_for_in_flight(|e| e.iter().any(|(k, _)| k == thread_id)).await;

    // Parallel turn on the SAME thread must NOT interrupt the primary — it
    // lives in the parallel lane while the primary stays in-flight.
    start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "pp-client",
        thread_id,
        "branch",
        None,
        None,
        None,
        Some("parallel".to_string()),
        ChatRequestMetadata::default(),
    )
    .await
    .expect("parallel turn should start");

    wait_for_parallel(|e| e.iter().any(|(_, t)| t == thread_id)).await;
    // Primary is still in-flight — the parallel send did not interrupt it.
    assert!(
        in_flight_entries_for_test(&crate::core::runtime::DispatchCtx::ambient())
            .await
            .iter()
            .any(|(k, _)| k == thread_id),
        "primary turn must remain in-flight alongside the parallel turn"
    );

    // A thread-level cancel tears down BOTH the primary and the parallel turn.
    cancel_chat_scoped(
        &crate::core::runtime::DispatchCtx::ambient(),
        "pp-client",
        thread_id,
        None,
    )
    .await
    .expect("cancel should succeed");
    wait_for_in_flight(|e| !e.iter().any(|(k, _)| k == thread_id)).await;
    wait_for_parallel(|e| !e.iter().any(|(_, t)| t == thread_id)).await;

    set_test_run_chat_task_block(None).await;
}

// ── #3714: session-expired arm (must precede `auth_error`) ──────────────
#[test]
fn classify_session_expired_sentinel_routes_to_signin_not_generic() {
    for raw in [
        "SESSION_EXPIRED: backend session not active — sign in to resume LLM work",
        "SESSION_EXPIRED: backend session token expired locally — re-authentication required",
        "no backend session token; run auth_store_session first",
    ] {
        let c = classify_inference_error(raw);
        assert_eq!(c.error_type, "session_expired", "raw={raw:?}");
        assert!(!c.retryable, "session-expiry is not retryable: {raw:?}");
        assert_ne!(
            c.message,
            generic_inference_error_user_message(),
            "must not be the generic catch-all: {raw:?}"
        );
    }
}

#[test]
fn classify_session_expired_claims_managed_backend_401_invalid_token_before_auth_error() {
    // The OpenHuman backend 401 "Invalid token" envelope contains "401", which
    // the `auth_error` arm would otherwise claim ("check your API key") — wrong
    // for managed-backend users. The session arm must win.
    let c = classify_inference_error(
        "OpenHuman API error (401 Unauthorized): {\"error\":\"Invalid token\"}",
    );
    assert_eq!(c.error_type, "session_expired");
}

#[test]
fn classify_byo_provider_401_stays_auth_error_not_session_expired() {
    // A BYO provider's own 401 (user's API key) must NOT be swallowed by the
    // session arm — it stays actionable as `auth_error`.
    let c = classify_inference_error(
        "OpenAI API error (401 Unauthorized): {\"error\":{\"message\":\"invalid_api_key\"}}",
    );
    assert_eq!(c.error_type, "auth_error");
}

// ── #3714: transport-drop arm (bucket #1, was the generic catch-all) ─────
#[test]
fn classify_connection_drop_routes_to_network_retryable() {
    for raw in [
        "error sending request for url (https://api.tinyhumans.ai/openai/v1/chat/completions): \
         connection closed before message completed",
        "request or response body error: unexpected end of file",
        // Raw mid-stream SSE drop: managed backend leaves OFF the errorCode, so
        // it reaches the ladder as a streaming error with a transport body.
        "OpenHuman streaming API error: error reading a body from connection: \
         end of file before message length reached",
    ] {
        let c = classify_inference_error(raw);
        assert_eq!(c.error_type, "network", "raw={raw:?}");
        assert!(c.retryable, "transport drop is retryable: {raw:?}");
        assert_ne!(
            c.message,
            generic_inference_error_user_message(),
            "raw={raw:?}"
        );
    }
}

#[test]
fn classify_managed_sse_badrequest_not_misread_as_network() {
    // A managed 400 frame carries errorCode → must stay provider_request_rejected
    // (claimed by the errorCode short-circuit before the transport arm).
    let c = classify_inference_error(
        "OpenHuman streaming API error: {\"error\":{\"message\":\"Message has tool role, \
         but there was no previous assistant message with a tool call!\",\
         \"type\":\"stream_error\",\"errorCode\":\"BAD_REQUEST\"}}",
    );
    assert_eq!(c.error_type, "provider_request_rejected");
}

#[test]
fn classify_timeout_not_shadowed_by_network_arm() {
    let c = classify_inference_error("request timed out while reading response");
    assert_eq!(c.error_type, "timeout");
}

// ── #3714: poisoned-history 400 gets the "we cleared it, resend" copy ────
#[test]
fn classify_managed_tool_ordering_400_gets_cleared_resend_copy() {
    // Managed backend `validateToolMessageOrdering` rejection (orphaned tool
    // message) arrives as a BAD_REQUEST SSE frame — must read "we cleared it,
    // send again" (not "try a different model") and be retryable, since the
    // de-poison guard already evicted the bad warm session.
    let c = classify_inference_error(
        "OpenHuman streaming API error: {\"error\":{\"message\":\"Message at index 3 has role \
         'tool' but is not preceded by an assistant message with a matching tool_call\",\
         \"type\":\"stream_error\",\"errorCode\":\"BAD_REQUEST\"}}",
    );
    assert_eq!(c.error_type, "provider_request_rejected");
    assert!(c.retryable, "post-eviction resend works → retryable");
    assert!(c.message.contains("cleared it"), "got: {}", c.message);
    assert!(!c.message.contains("different model"), "got: {}", c.message);
}

#[test]
fn classify_byo_tool_ordering_400_gets_cleared_resend_copy() {
    let c = classify_inference_error(
        "OpenAI API error (400 Bad Request): {\"error\":{\"message\":\"Invalid parameter: \
         messages with role 'tool' must be a response to a preceding message with 'tool_calls'.\"}}",
    );
    assert_eq!(c.error_type, "provider_request_rejected");
    assert!(c.retryable);
    assert!(c.message.contains("cleared it"), "got: {}", c.message);
}

#[test]
fn classify_genuine_param_400_keeps_model_mismatch_copy_not_glitch() {
    // A real model/param 400 (no tool-ordering signature) must NOT get the
    // "we cleared it" copy — resending the same params fails again.
    let c = classify_inference_error(
        "custom_openai API error (400 Bad Request): {\"error\":{\"message\":\
         \"Unsupported value: 'temperature' must be 1 for this model\"}}",
    );
    assert_eq!(c.error_type, "provider_request_rejected");
    assert!(!c.retryable, "param mismatch is not retryable");
    assert!(!c.message.contains("cleared it"), "got: {}", c.message);
}

/// The Stop button must reach a thread's detached background sub-agents. They
/// run on their own tasks and drop the spawning turn's cancellation, so before
/// this the parent turn stopped but the child kept working — and its result
/// later started a fresh delivery turn on the thread. A scoped cancel (one named
/// request) must leave them alone.
#[tokio::test]
async fn unscoped_cancel_stops_the_threads_detached_subagents() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let _registry = crate::config::TEST_ENV_LOCK.lock().await;
    use crate::agent::orchestration::running_subagents;

    let thread_id = "stop-detached-subagent-thread";
    let workspace = tempfile::tempdir().expect("workspace");
    let child = tokio::spawn(std::future::pending::<()>());
    let (_status_tx, status_rx) = running_subagents::status_channel();
    running_subagents::register(
        &crate::core::runtime::DispatchCtx::ambient(),
        "task-web-stop-1".into(),
        "researcher".into(),
        "session-web-stop".into(),
        None,
        None,
        workspace.path().to_path_buf(),
        Some(thread_id.into()),
        std::sync::Arc::new(tinyagents_harness::run_queue::RunQueue::new()),
        child.abort_handle(),
        status_rx,
    )
    .expect("fixture registration admitted");

    // A scoped cancel for some other request is not a Stop: the child lives.
    let scoped = channel_web_cancel(
        &crate::core::runtime::DispatchCtx::ambient(),
        "stop-client",
        thread_id,
        Some("req-unrelated"),
    )
    .await
    .expect("scoped cancel");
    assert_eq!(scoped.value["cancelled"], serde_json::json!(false));
    assert_eq!(scoped.value["subagents_cancelled"], serde_json::json!(0));
    assert!(
        !child.is_finished(),
        "scoped cancel must not stop sub-agents"
    );

    // The Stop button: no turn in flight, but the detached child is stopped.
    let stop = channel_web_cancel(
        &crate::core::runtime::DispatchCtx::ambient(),
        "stop-client",
        thread_id,
        None,
    )
    .await
    .expect("stop");
    assert_eq!(stop.value["cancelled"], serde_json::json!(true));
    assert_eq!(stop.value["request_id"], serde_json::Value::Null);
    assert_eq!(stop.value["subagents_cancelled"], serde_json::json!(1));
    let joined = tokio::time::timeout(std::time::Duration::from_secs(2), child)
        .await
        .expect("aborted child finishes promptly");
    assert!(joined.expect_err("child aborted").is_cancelled());
}

/// The terminal event (`chat_done` or `chat_error`) for `request_id`.
async fn terminal_event(
    rx: &mut tokio::sync::broadcast::Receiver<crate::web_chat::WebChannelEvent>,
    request_id: &str,
) -> crate::web_chat::WebChannelEvent {
    loop {
        let event = rx.recv().await.expect("event stream should stay open");
        if event.request_id == request_id
            && matches!(event.event.as_str(), "chat_done" | "chat_error")
        {
            return event;
        }
    }
}

/// A graceful, retryable `turn_timeout` whose user copy hides the marker.
fn assert_turn_timeout(event: &crate::web_chat::WebChannelEvent) {
    assert_eq!(event.event, "chat_error", "{event:?}");
    assert_eq!(event.error_type.as_deref(), Some("turn_timeout"));
    assert_eq!(event.error_retryable, Some(true));
    let message = event.message.clone().unwrap_or_default();
    assert!(
        !message.contains("openhuman_turn_wall_clock_timeout"),
        "internal marker must not leak into user copy: {message}"
    );
}
