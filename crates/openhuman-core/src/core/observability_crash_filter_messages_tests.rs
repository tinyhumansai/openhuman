use super::*;

#[cfg(feature = "crash-reporting")]
#[test]
fn max_iterations_filter_matches_message_path() {
    // `report_error_message` calls `sentry::capture_message`, which
    // populates `event.message`. The filter must see the canonical
    // phrase on that field path.
    let event = event_with_message("Agent exceeded maximum tool iterations (8)");
    assert!(is_max_iterations_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn max_iterations_filter_matches_exception_path() {
    // sentry-tracing with attach_stacktrace=true populates the
    // exception list instead of (or in addition to) `event.message`.
    // Filter must still catch the noise.
    let event = event_with_exception_value(
        "agent.run_single failed: Agent exceeded maximum tool iterations (10)",
    );
    assert!(is_max_iterations_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn max_iterations_filter_keeps_unrelated_events() {
    assert!(!is_max_iterations_event(&event_with_message(
        "provider returned 503"
    )));
    assert!(!is_max_iterations_event(&event_with_message("")));
    assert!(!is_max_iterations_event(&sentry::protocol::Event::default()));
}

// ── is_channel_message_not_found_event (TAURI-R7) ────────────────────────

#[cfg(feature = "crash-reporting")]
#[test]
fn channel_message_not_found_filter_matches_patch() {
    // Canonical TAURI-R7 shape: PATCH 404 on a channel-message path.
    assert!(is_channel_message_not_found_event(
        &channel_message_404_event("PATCH")
    ));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn channel_message_not_found_filter_matches_delete() {
    assert!(is_channel_message_not_found_event(
        &channel_message_404_event("DELETE")
    ));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn channel_message_not_found_filter_ignores_get_404() {
    // GET 404 on a channel-message path is NOT an expected state — must keep Sentry signal.
    assert!(!is_channel_message_not_found_event(
        &channel_message_404_event("GET")
    ));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn channel_message_not_found_filter_ignores_non_channel_path() {
    let mut event = channel_message_404_event("PATCH");
    event.message = Some("PATCH /auth/profile failed (404); response_body_len=42".to_string());
    assert!(!is_channel_message_not_found_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn channel_message_not_found_filter_ignores_wrong_status() {
    let mut event = channel_message_404_event("PATCH");
    event.tags.insert("status".into(), "403".into());
    assert!(!is_channel_message_not_found_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn channel_message_not_found_filter_ignores_wrong_domain() {
    let mut event = channel_message_404_event("PATCH");
    event.tags.insert("domain".into(), "channels".into());
    assert!(!is_channel_message_not_found_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn channel_message_not_found_filter_matches_exception_path() {
    // sentry-tracing with attach_stacktrace=true populates exception list.
    let mut event = sentry::protocol::Event::default();
    event.tags.insert("domain".into(), "backend_api".into());
    event.tags.insert("failure".into(), "non_2xx".into());
    event.tags.insert("status".into(), "404".into());
    event.tags.insert("method".into(), "PATCH".into());
    event.exception = vec![sentry::protocol::Exception {
        value: Some("PATCH /channels/discord/messages/abc failed (404): Not Found".to_string()),
        ..Default::default()
    }]
    .into();
    assert!(is_channel_message_not_found_event(&event));
}

// ── LoopbackUnavailable (TAURI-R5, TAURI-R6) ─────────────────────────────

/// Verbatim body shape from OPENHUMAN-TAURI-R5 (~2.5k events): the
/// `integrations.get` site reaches the embedded core's `127.0.0.1:18474`
/// listener during the boot window and reqwest's source chain renders as
/// `error sending request for url (…) → client error (Connect) → tcp
/// connect error → Connection refused (os error 61)`.
const R5_BODY: &str = "error sending request for url \
    (http://127.0.0.1:18474/agent-integrations/composio/connections) \
    → client error (Connect) → tcp connect error → Connection refused (os error 61)";

/// Verbatim body shape from OPENHUMAN-TAURI-R6 (~2.5k events): the same
/// transport failure as R5, re-wrapped one frame up by the composio
/// op-layer and re-emitted at the `rpc.invoke_method` site so it lands in
/// Sentry under `domain=rpc` instead of `domain=integrations`.
const R6_BODY: &str = "[composio] list_connections failed: \
    GET http://127.0.0.1:18474/agent-integrations/composio/connections failed: \
    error sending request for url \
    (http://127.0.0.1:18474/agent-integrations/composio/connections) \
    → client error (Connect) → tcp connect error → Connection refused (os error 61)";

#[test]
fn classifies_r5_loopback_connect_refused_as_loopback_unavailable() {
    assert_eq!(
        expected_error_kind(R5_BODY),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "R5 body must classify as LoopbackUnavailable, not the broader NetworkUnreachable bucket"
    );
}

#[test]
fn classifies_r6_rpc_wrapped_loopback_connect_refused_as_loopback_unavailable() {
    assert_eq!(
        expected_error_kind(R6_BODY),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "R6 body (rpc.invoke_method re-wrap) must classify as LoopbackUnavailable"
    );
}

#[test]
fn classifies_loopback_connect_refused_across_platforms() {
    // Linux WSL / native: os error 111. Windows WSAECONNREFUSED: 10061.
    // Both must classify so the matcher works regardless of where the
    // user's desktop happens to be running.
    for raw in [
        "error sending request for url (http://127.0.0.1:18474/x) \
         → tcp connect error → Connection refused (os error 111)",
        "error sending request for url (http://localhost:18474/x) \
         → tcp connect error → Connection refused (os error 10061)",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::LoopbackUnavailable),
            "should classify as LoopbackUnavailable across platforms: {raw}"
        );
    }
}

#[test]
fn loopback_unavailable_precedence_over_network_unreachable() {
    // Precedence guard: a loopback `Connection refused (os error 61)`
    // body would ALSO match `is_network_unreachable_message` because the
    // broader matcher catches both `error sending request for url` and
    // `connection refused`. The ladder must route through the
    // loopback-specific bucket first so the two error classes stay
    // distinguishable in Sentry.
    let kind = expected_error_kind(R5_BODY);
    assert_eq!(kind, Some(ExpectedErrorKind::LoopbackUnavailable));
    assert_ne!(kind, Some(ExpectedErrorKind::NetworkUnreachable));
}

#[test]
fn does_not_classify_loopback_url_with_different_error_class_as_loopback() {
    // A real upstream HTTP failure that happens to hit a developer's
    // local proxy on `127.0.0.1:` (e.g. `mitmproxy`, `Charles`,
    // `ngrok http`) must NOT be silenced as loopback noise — the body
    // shape is a 503 status, not a transport-level connect-refused, and
    // is actionable for Sentry.
    let raw = "Backend returned 503 Service Unavailable for GET \
               http://127.0.0.1:8080/agent-integrations/composio/connections: \
               upstream timed out";
    assert!(
        !matches!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::LoopbackUnavailable)
        ),
        "loopback URL with non-transport error must not classify as LoopbackUnavailable"
    );
}

#[test]
fn does_not_classify_non_loopback_connect_refused_as_loopback() {
    // A `Connection refused` against a non-loopback host (DNS resolved
    // to a remote IP, ISP-level block, captive portal) must fall
    // through to `NetworkUnreachable`, not into the loopback bucket.
    let raw = "error sending request for url \
               (https://api.tinyhumans.ai/agent-integrations/composio/connections) \
               → tcp connect error → Connection refused (os error 61)";
    assert_eq!(
        expected_error_kind(raw),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

/// Verbatim body from TAURI-RUST-12K (2802 events / 29 users): a cron
/// agent job hits a local LM Studio server (`localhost:1234`) that isn't
/// running, on a zh-CN Windows host — so the `WSAECONNREFUSED` text is
/// localized and the English "connection refused" prefix is absent, leaving
/// only `(os error 10061)` behind the transport-stable `tcp connect error`
/// marker. The locale-independent arm must still route it to the loopback
/// bucket rather than leaking to the broad `NetworkUnreachable`.

#[test]
fn classifies_localized_loopback_connect_refused_as_loopback_unavailable() {
    let raw = "error sending request for url \
               (http://localhost:1234/v1/chat/completions): client error (Connect): \
               tcp connect error: 由于目标计算机积极拒绝，无法连接。 (os error 10061)";
    assert_eq!(
        expected_error_kind(raw),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "localized WSAECONNREFUSED loopback body must classify as LoopbackUnavailable"
    );
}

#[test]
fn does_not_classify_loopback_timeout_as_loopback() {
    // A loopback *timeout* (WSAETIMEDOUT os error 10060, not the
    // connect-refused 10061) shares the `tcp connect error` marker but is
    // a distinct failure class — it must NOT be swallowed by the
    // connect-refused loopback arm.
    let raw = "error sending request for url \
               (http://localhost:1234/v1/chat/completions): \
               tcp connect error: connection timed out (os error 10060)";
    assert!(
        !matches!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::LoopbackUnavailable)
        ),
        "loopback timeout must not classify as LoopbackUnavailable"
    );
}

#[test]
fn is_local_provider_unreachable_message_wraps_loopback_matcher() {
    let localized = "error sending request for url \
                     (http://localhost:1234/v1/chat/completions): client error (Connect): \
                     tcp connect error: 由于目标计算机积极拒绝，无法连接。 (os error 10061)";
    assert!(is_local_provider_unreachable_message(localized));
    // Remote refused must not match (loopback-only, keeps real outages visible).
    assert!(!is_local_provider_unreachable_message(
        "error sending request for url (https://api.tinyhumans.ai/x) \
         → tcp connect error → Connection refused (os error 61)"
    ));
}

#[test]
fn loopback_matcher_requires_both_host_and_errno_anchors() {
    // Defense against the matcher being too eager: bodies that satisfy
    // only one of the two conjunctive anchors must not classify into the
    // loopback bucket. They may still demote via the broader
    // `NetworkUnreachable` matcher — that is the correct fall-through —
    // but the bucket must stay distinct so Sentry's "what class is
    // spiking?" signal is preserved.
    let loopback_host_no_errno =
        "doctor: probed 127.0.0.1:18474 and got connection refused without errno detail";
    assert_ne!(
        expected_error_kind(loopback_host_no_errno),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "loopback host without `(os error N)` errno must not classify as LoopbackUnavailable"
    );

    let errno_no_loopback_host = "note: connection refused (os error 61) on retry";
    assert_ne!(
        expected_error_kind(errno_no_loopback_host),
        Some(ExpectedErrorKind::LoopbackUnavailable),
        "errno without loopback host anchor must not classify as LoopbackUnavailable"
    );
}

#[test]
fn report_error_or_expected_routes_r5_r6_through_expected_path() {
    // Smoke test: both verbatim Sentry bodies flow through
    // `report_error_or_expected` without panicking. The classifier
    // routes them to `report_expected_message` (debug breadcrumb,
    // metadata-only) instead of `report_error_message`
    // (`sentry::capture_message` at error level). We can't observe the
    // Sentry hub from this test, but exercising the call path catches
    // any future regression that re-introduces a panic or mis-types
    // the arm.
    report_error_or_expected(
        R5_BODY,
        "integrations",
        "get",
        &[
            ("path", "/agent-integrations/composio/connections"),
            ("failure", "transport"),
        ],
    );
    report_error_or_expected(
        R6_BODY,
        "rpc",
        "invoke_method",
        &[("method", "openhuman.composio_list_connections")],
    );
}

#[test]
fn classifies_channel_supervisor_restart_english_discord_gateway() {
    // TAURI-RUST-15 (~11.4k events / 14d on self-hosted `tauri-rust`):
    // verbatim wrapper from `channels::runtime::supervision::spawn_supervised_listener`
    // around the Discord gateway transport error. The English body
    // would otherwise match `is_network_unreachable_message` (which
    // demotes to `warn!` — still a Sentry event); the supervisor
    // wrap precedence routes it to `ChannelSupervisorRestart`
    // (info-only breadcrumb).
    let body = "Channel discord error: error sending request for url \
                (https://discord.com/api/v10/gateway/bot); restarting";
    assert_eq!(
        expected_error_kind(body),
        Some(ExpectedErrorKind::ChannelSupervisorRestart)
    );
}

#[test]
fn classifies_channel_supervisor_restart_chinese_windows_wsaetimedout() {
    // TAURI-RUST-BB (~815 events / 14d): same supervisor wrapper,
    // OS-localized inner WSAETIMEDOUT body on Chinese Windows. The
    // English-only `is_network_unreachable_message` anchors miss
    // this inner message, so without the language-agnostic
    // supervisor matcher it would escape classification entirely
    // and emit a full Sentry error. The wrapper-anchored predicate
    // catches it regardless of OS locale.
    let body = "Channel discord error: IO error: \
                由于连接方在一段时间后没有正确答复或连接的主机没有反应，连接尝试失败。 \
                (os error 10060); restarting";
    assert_eq!(
        expected_error_kind(body),
        Some(ExpectedErrorKind::ChannelSupervisorRestart)
    );
}

#[test]
fn channel_supervisor_restart_matches_multiple_channel_names() {
    // The wrapper format is `"Channel <name> error: <inner>; restarting"`.
    // The name slot varies by provider (discord, slack, telegram,
    // whatsapp, gmessages, …). The matcher must classify all of them —
    // language-agnostic, name-agnostic.
    for raw in [
        "Channel slack error: gateway disconnect; restarting",
        "Channel telegram error: tls handshake eof; restarting",
        "Channel whatsapp error: connection reset by peer (os error 54); restarting",
        "Channel gmessages error: WebSocket connect: HTTP error: 502 Bad Gateway; restarting",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ChannelSupervisorRestart),
            "should classify as channel-supervisor-restart: {raw}"
        );
    }
}

#[test]
fn channel_supervisor_restart_precedence_over_network_unreachable() {
    // Pin the precedence: a supervisor-wrap body that ALSO contains
    // the canonical `"error sending request for url"` anchor (which
    // would by itself classify as `NetworkUnreachable`) MUST route
    // to `ChannelSupervisorRestart`. The supervisor's own backoff
    // handles the condition; `NetworkUnreachable` would demote to
    // `warn!` (still a Sentry event), whereas
    // `ChannelSupervisorRestart` demotes to `info!` (no event).
    let body = "Channel discord error: error sending request for url \
                (https://discord.com/api/v10/gateway/bot); restarting";
    let kind = expected_error_kind(body);
    assert_eq!(kind, Some(ExpectedErrorKind::ChannelSupervisorRestart));
    assert_ne!(kind, Some(ExpectedErrorKind::NetworkUnreachable));
}

#[test]
fn channel_supervisor_restart_does_not_classify_unrelated_restart_notes() {
    // Defense against the matcher being too eager: bodies that
    // contain `"; restarting"` but NOT the `"Channel <name> error:"`
    // preamble must NOT classify — those are generic restart logs
    // from other subsystems where Sentry signal may still be
    // actionable. The matcher requires all three anchors together
    // (`"channel "` prefix + `" error:"` separator + `"; restarting"`
    // trailer).
    for raw in [
        // No `Channel <name>` preamble.
        "systemd: docker.service; restarting",
        // No `Channel <name>` preamble even though `; restarting`
        // appears.
        "Connection refused; restarting",
        // The string `channel` appears but not as the leading
        // `"Channel <name> error:"` wrapper — must not classify.
        "channels::runtime::dispatch failed: error: provider exhausted; restarting",
        // The wrapper prefix is present but the trailer is not —
        // a half-formed log line must not classify.
        "Channel discord error: gateway disconnect",
    ] {
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ChannelSupervisorRestart),
            "must NOT classify as channel-supervisor-restart: {raw}"
        );
    }
}

#[test]
fn report_error_or_expected_routes_channel_supervisor_restart_through_expected_path() {
    // Smoke test: the verbatim TAURI-RUST-15 Sentry body flows through
    // `report_error_or_expected` without panicking. The classifier
    // routes it to `report_expected_message` (info breadcrumb) instead
    // of `report_error_message` (`sentry::capture_message` at error
    // level). We can't observe the Sentry hub from this test, but
    // exercising the call path catches any future regression that
    // re-introduces a panic or mis-types the arm.
    report_error_or_expected(
        "Channel discord error: error sending request for url \
         (https://discord.com/api/v10/gateway/bot); restarting",
        "channels",
        "supervised_listener",
        &[("channel", "discord")],
    );
}

// ── #870 managed-backend errorCode Sentry ownership (F2/F4/F7/F8) ──

#[test]
fn expected_kind_demotes_every_backend_owned_error_code() {
    for (status, code) in [
        ("429", "RATE_LIMITED"),
        ("402", "USER_INSUFFICIENT_CREDITS"),
        ("503", "UPSTREAM_UNAVAILABLE"),
        ("404", "MODEL_UNAVAILABLE"),
        ("400", "BAD_REQUEST"),
        ("500", "INTERNAL_ERROR"),
    ] {
        let body = managed_body(status, code);
        assert_eq!(
            expected_error_kind(&body),
            Some(ExpectedErrorKind::BackendErrorCodeOwned),
            "errorCode={code} must be backend-owned (no FE Sentry)"
        );
    }

    // Client-guard-leak codes page (None = capture), even with realistic
    // explanatory text that a later substring matcher would otherwise
    // re-demote into a suppressed bucket.
    let payload = "OpenHuman API error (413 Payload Too Large): \
         {\"error\":{\"errorCode\":\"PAYLOAD_TOO_LARGE\",\"message\":\"request entity too large\"}}";
    register_backend_classifier_fixture(
        payload,
        reportable_fixture(BackendErrorKind::PayloadTooLarge, false),
    );
    assert_eq!(
        expected_error_kind(payload),
        None,
        "PAYLOAD_TOO_LARGE is a client guard leak and must page"
    );

    // This body's message matches `is_context_window_exceeded_message`, so
    // without the early guard-leak bypass it would re-demote to the
    // suppressed `ContextWindowExceeded` bucket (CodeRabbit).
    let context = "OpenHuman API error (400 Bad Request): \
         {\"error\":{\"errorCode\":\"CONTEXT_LENGTH_EXCEEDED\",\"message\":\"This model's maximum context length is 128000 tokens, however you requested more\"}}";
    register_backend_classifier_fixture(
        context,
        reportable_fixture(BackendErrorKind::ContextLengthExceeded, false),
    );
    assert_eq!(
        expected_error_kind(context),
        None,
        "CONTEXT_LENGTH_EXCEEDED must page even with context-window wording"
    );

    // Proof the bypass is load-bearing, not a no-op: this body's text DOES
    // match the context-window matcher, so without the early guard-leak
    // bypass `expected_error_kind` would have re-demoted it to the
    // suppressed `ContextWindowExceeded` bucket.
    assert!(
        tinyinference_llm::failure::is_context_window_exceeded_message(context),
        "test body must actually trigger the matcher the bypass guards against"
    );
}

#[test]
fn expected_kind_lets_malformed_bad_request_page() {
    // F8: the one errorCode case that still pages — the backend flagged a
    // client-built payload as unparseable. `expected_error_kind` must NOT
    // classify it as expected, so capture proceeds.
    let body = "OpenHuman API error (400 Bad Request): \
         {\"error\":{\"errorCode\":\"BAD_REQUEST\",\"malformed\":true}}";
    register_backend_classifier_fixture(
        body,
        reportable_fixture(BackendErrorKind::BadRequest, true),
    );
    assert_eq!(expected_error_kind(body), None);
}

#[test]
fn expected_kind_ignores_byo_errors_with_no_code() {
    // A BYO 401 carries no errorCode — it must NOT be swallowed by the
    // backend-owned branch (it routes through its own matchers / capture).
    let body = "openai API error (401 Unauthorized): \
         {\"error\":{\"message\":\"Incorrect API key provided\"}}";
    assert_ne!(
        expected_error_kind(body),
        Some(ExpectedErrorKind::BackendErrorCodeOwned)
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn expected_kind_ignores_byo_errors_that_carry_an_error_code_token() {
    // CodeRabbit: a BYO / direct-provider envelope whose body happens to
    // carry an `errorCode`-shaped field must NOT be demoted — the
    // managed-envelope gate keeps it reaching Sentry.
    let body = "custom_openai API error (500 Internal Server Error): \
         {\"error\":{\"errorCode\":\"INTERNAL_ERROR\"}}";
    assert_ne!(
        expected_error_kind(body),
        Some(ExpectedErrorKind::BackendErrorCodeOwned)
    );
    let event = event_with_message(body);
    assert!(
        !is_backend_error_code_event(&event),
        "BYO body with an errorCode token must not be dropped by before_send"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn before_send_filter_drops_backend_owned_error_code_events() {
    for code in [
        "RATE_LIMITED",
        "UPSTREAM_UNAVAILABLE",
        "MODEL_UNAVAILABLE",
        "INTERNAL_ERROR",
        "BAD_REQUEST",
    ] {
        let event = event_with_message(&managed_body("500", code));
        assert!(
            is_backend_error_code_event(&event),
            "errorCode={code} event must be dropped by before_send"
        );
    }

    // Client-guard-leak codes survive before_send and page (the client
    // should have caught the limit before sending) — including a realistic
    // CONTEXT_LENGTH_EXCEEDED body whose wording matches the context-window
    // substring matcher (the filter keys on the errorCode, not the text).
    let payload = "OpenHuman API error (413 Payload Too Large): \
         {\"error\":{\"errorCode\":\"PAYLOAD_TOO_LARGE\",\"message\":\"request entity too large\"}}";
    let context = "OpenHuman API error (400 Bad Request): \
         {\"error\":{\"errorCode\":\"CONTEXT_LENGTH_EXCEEDED\",\"message\":\"This model's maximum context length is 128000 tokens\"}}";
    register_backend_classifier_fixture(
        payload,
        reportable_fixture(BackendErrorKind::PayloadTooLarge, false),
    );
    register_backend_classifier_fixture(
        context,
        reportable_fixture(BackendErrorKind::ContextLengthExceeded, false),
    );
    for body in [payload, context] {
        let event = event_with_message(body);
        assert!(
            !is_backend_error_code_event(&event),
            "client guard leak must survive before_send: {body}"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn before_send_filter_keeps_malformed_bad_request_event() {
    let body = "OpenHuman API error (400 Bad Request): \
         {\"error\":{\"errorCode\":\"BAD_REQUEST\",\"malformed\":true}}";
    register_backend_classifier_fixture(
        body,
        reportable_fixture(BackendErrorKind::BadRequest, true),
    );
    let event = event_with_message(body);
    assert!(
        !is_backend_error_code_event(&event),
        "malformed BAD_REQUEST must survive before_send and page (F8)"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn before_send_filter_matches_error_code_in_exception_value() {
    let event = event_with_exception_value(&managed_body("500", "INTERNAL_ERROR"));
    assert!(is_backend_error_code_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_provider_transport_filter_drops_flaky_network_blips() {
    // F7: a streaming transport timeout/reset under
    // domain=llm_provider, failure=transport is recovered by
    // retry/fallback — drop it.
    for phrase in [
        "error sending request for url (https://api.tinyhumans.ai/v1/chat): \
         operation timed out",
        "connection reset by peer",
        "tls handshake eof",
    ] {
        for operation in ["stream_chat", "stream_chat_history"] {
            let event = event_with_tags_and_message(
                &[
                    ("domain", "llm_provider"),
                    ("failure", "transport"),
                    ("operation", operation),
                ],
                phrase,
            );
            assert!(
                is_transient_provider_transport_failure(&event),
                "transient transport phrase must be dropped: {phrase} ({operation})"
            );
        }
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_provider_transport_filter_keeps_non_transient_transport() {
    // A genuine, non-transient transport failure (e.g. an unexpected
    // protocol error) is NOT a flaky-network blip — keep paging.
    let event = event_with_tags_and_message(
        &[
            ("domain", "llm_provider"),
            ("failure", "transport"),
            ("operation", "stream_chat"),
        ],
        "invalid URL scheme: unsupported protocol",
    );
    assert!(!is_transient_provider_transport_failure(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_provider_transport_filter_scoped_to_streaming_operations() {
    // CodeRabbit: a NON-streaming llm_provider transport failure with the
    // same domain/failure tags but a different operation must keep paging
    // — the filter is intentionally scoped to the streaming emits.
    let event = event_with_tags_and_message(
        &[
            ("domain", "llm_provider"),
            ("failure", "transport"),
            ("operation", "chat_completions"),
        ],
        "operation timed out",
    );
    assert!(
        !is_transient_provider_transport_failure(&event),
        "non-streaming llm_provider transport must not be suppressed"
    );

    // And with no operation tag at all (older/foreign emit) — keep paging.
    let no_op = event_with_tags_and_message(
        &[("domain", "llm_provider"), ("failure", "transport")],
        "operation timed out",
    );
    assert!(!is_transient_provider_transport_failure(&no_op));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_provider_transport_filter_scoped_to_llm_provider() {
    // Same shape under a different domain must not be claimed by this
    // provider-scoped filter.
    let event = event_with_tags_and_message(
        &[
            ("domain", "backend_api"),
            ("failure", "transport"),
            ("operation", "stream_chat"),
        ],
        "operation timed out",
    );
    assert!(!is_transient_provider_transport_failure(&event));
}
