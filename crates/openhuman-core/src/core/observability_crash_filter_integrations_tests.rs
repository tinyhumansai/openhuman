use super::*;

#[cfg(feature = "crash-reporting")]
#[test]
fn integrations_filter_drops_transient_statuses() {
    for status in TRANSIENT_HTTP_STATUSES {
        let event = event_with_tags(&[
            ("domain", "integrations"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_integrations_failure(&event),
            "integrations status {status} must be classified as transient"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn integrations_filter_drops_transient_transport_phrases() {
    for phrase in TRANSIENT_TRANSPORT_PHRASES {
        let event = event_with_tags_and_message(
            &[("domain", "integrations"), ("failure", "transport")],
            &format!("GET /agent-integrations/tools failed: {phrase}"),
        );
        assert!(
            is_transient_integrations_failure(&event),
            "integrations transport phrase {phrase} must be classified as transient"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn integrations_filter_keeps_non_transient_failures() {
    for status in ["404", "500"] {
        let event = event_with_tags(&[
            ("domain", "integrations"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_integrations_failure(&event),
            "integrations status {status} must stay visible"
        );
    }

    // Sibling-domain check: composio op-layer events MUST be silenced
    // by the integrations filter — composio routes through the same
    // `IntegrationClient` so the failure shape is identical, but
    // op-level reporters that wrap and re-emit with their own domain
    // tag would otherwise escape (OPENHUMAN-TAURI-35 / -2H).
    let scheduler_domain = event_with_tags(&[
        ("domain", "scheduler"),
        ("failure", "non_2xx"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_integrations_failure(&scheduler_domain),
        "domain scoping must keep unrelated transient-shaped events visible"
    );

    let non_matching_transport = event_with_tags_and_message(
        &[("domain", "integrations"), ("failure", "transport")],
        "GET /agent-integrations/tools failed: invalid certificate",
    );
    assert!(
        !is_transient_integrations_failure(&non_matching_transport),
        "transport failures without an allowlisted phrase must stay visible"
    );
}

/// TAURI-RUST-CGE: skill-install fetch 4xx (a missing/renamed catalog
/// `SKILL.md`) is expected user-input state — the before_send net must drop
/// it, while a genuine 5xx remote failure and unrelated domains stay
/// reportable.
#[cfg(feature = "crash-reporting")]
#[test]
fn skills_install_client_error_filter_drops_4xx_keeps_5xx() {
    // 4xx (esp. 404/410) = missing skill / wrong URL → dropped.
    for status in ["400", "403", "404", "410", "429"] {
        let event = event_with_tags(&[
            ("domain", "skills"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_skills_install_client_error_event(&event),
            "skills install 4xx {status} must be dropped"
        );
    }

    // 5xx = genuine remote failure → stays a Sentry signal.
    for status in ["500", "502", "503"] {
        let event = event_with_tags(&[
            ("domain", "skills"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_skills_install_client_error_event(&event),
            "skills install 5xx {status} must stay reportable"
        );
    }

    // Domain scoping: a 4xx in another domain must not be touched here.
    let other_domain = event_with_tags(&[
        ("domain", "backend_api"),
        ("failure", "non_2xx"),
        ("status", "404"),
    ]);
    assert!(
        !is_skills_install_client_error_event(&other_domain),
        "non-skills 4xx must not be swallowed by the skills filter"
    );

    // A skills event without the non_2xx failure marker (e.g. a transport
    // failure) must not be dropped by this status-scoped filter.
    let skills_transport = event_with_tags(&[("domain", "skills"), ("failure", "transport")]);
    assert!(
        !is_skills_install_client_error_event(&skills_transport),
        "skills transport failures are out of scope for the 4xx filter"
    );
}

/// Skill-registry defects (a malformed catalog, a transport that broke its
/// contract) are not user input: neither skills filter may drop them.
#[cfg(feature = "crash-reporting")]
#[test]
fn skill_registry_defects_pass_the_skills_filters() {
    for failure in ["malformed", "transport_contract"] {
        let event = event_with_tags(&[
            ("domain", "skills"),
            ("operation", "registry"),
            ("failure", failure),
        ]);
        assert!(!is_skills_install_client_error_event(&event), "{failure}");
        assert!(!is_skill_install_user_fetch_failure(&event), "{failure}");
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn composio_domain_routes_through_integrations_filter() {
    // OPENHUMAN-TAURI-35 (~139 events) / -2H (~26 events):
    // `[composio] list_connections failed: Backend returned 502 …` —
    // composio op-layer wrappers (e.g. `composio_list_connections`) emit
    // errors under `domain="composio"` so the original
    // `domain="integrations"` filter let them through. Routing the
    // composio domain through the same transient classifier closes
    // that gap; the underlying transport / non_2xx semantics are
    // identical because both layers share the same `IntegrationClient`.
    for status in TRANSIENT_HTTP_STATUSES {
        let event = event_with_tags(&[
            ("domain", "composio"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_integrations_failure(&event),
            "composio status {status} must be classified as transient"
        );
    }

    // Transport-phrase variant — composio also surfaces reqwest
    // transport failures (timeouts, connection resets) once the op
    // wrapper has tagged the event with `failure=transport`.
    for phrase in TRANSIENT_TRANSPORT_PHRASES {
        let event = event_with_tags_and_message(
            &[("domain", "composio"), ("failure", "transport")],
            &format!("[composio] execute failed: {phrase}"),
        );
        assert!(
            is_transient_integrations_failure(&event),
            "composio transport phrase {phrase} must be classified as transient"
        );
    }

    // Non-transient composio statuses (404 / 500) must still surface —
    // actionable bugs even when reported under the composio domain.
    for status in ["404", "500"] {
        let event = event_with_tags(&[
            ("domain", "composio"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_integrations_failure(&event),
            "composio status {status} must stay visible"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn composio_list_connections_503_504_wrappers_stay_filtered() {
    for (status, reason) in [("503", "Service Unavailable"), ("504", "Gateway Timeout")] {
        let message = format!(
            "[composio] list_connections failed: Backend returned {status} {reason} \
             for GET /agent-integrations/composio/connections"
        );
        let event = event_with_tags_and_message(
            &[
                ("domain", "composio"),
                ("failure", "non_2xx"),
                ("status", status),
            ],
            &message,
        );
        assert!(
            is_transient_integrations_failure(&event),
            "wrapped composio list_connections {status} failures must be filtered"
        );
    }

    for (status, reason) in [("503", "Service Unavailable"), ("504", "Gateway Timeout")] {
        let message = format!(
            "Backend returned {status} {reason} \
             for GET /agent-integrations/composio/connections"
        );
        let event = event_with_tags_and_message(
            &[
                ("domain", "integrations"),
                ("failure", "non_2xx"),
                ("status", status),
            ],
            &message,
        );
        assert!(
            is_transient_integrations_failure(&event),
            "raw integrations list_connections {status} failures must be filtered"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn updater_transient_403_is_dropped() {
    let event = event_with_tags_and_message(
        &[
            ("domain", "update"),
            ("operation", "check_releases"),
            ("failure", "non_2xx"),
            ("status", "403"),
        ],
        "[observability] update.check_releases failed: GitHub API error: 403 Forbidden",
    );
    assert!(
        is_updater_transient_event(&event),
        "GitHub 403 updater checks are unactionable transient/rate-limit noise"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn updater_github_403_message_only_shapes_are_dropped() {
    for event in [
        event_with_message("GitHub API error: 403 Forbidden"),
        event_with_exception_value("GitHub API error: 403 Forbidden"),
    ] {
        assert!(
            is_updater_transient_event(&event),
            "message-only GitHub 403 updater failures must be filtered"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn updater_transient_502_is_dropped() {
    let event = event_with_tags_and_message(
        &[
            ("domain", "update.check_releases"),
            ("failure", "non_2xx"),
            ("status", "502"),
        ],
        "GitHub API error: 502 Bad Gateway",
    );
    assert!(
        is_updater_transient_event(&event),
        "GitHub 5xx updater checks must be filtered as transient"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn updater_real_panic_still_reported() {
    let event = event_with_tags_and_message(
        &[("domain", "update"), ("operation", "check_releases")],
        "thread 'main' panicked at crates/openhuman-core/src/platform/update/core.rs: index out of bounds",
    );
    assert!(
        !is_updater_transient_event(&event),
        "update-domain events without a transient updater shape must still reach Sentry"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn updater_endpoint_non_success_message_is_dropped() {
    // TAURI-RUST-CD (~151 events / 9 days, Windows): `tauri-plugin-updater`
    // logs `update endpoint did not respond with a successful status code`
    // (updater.rs) on any non-2xx response and discards the status, so the
    // captured event has NO `domain`/`status` tag — only the bare message.
    // It can therefore only be matched via the message fast-path.
    assert!(is_updater_transient_message(
        "update endpoint did not respond with a successful status code"
    ));

    let event = event_with_tags_and_message(
        &[],
        "update endpoint did not respond with a successful status code",
    );
    assert!(
        is_updater_transient_event(&event),
        "the plugin's status-blind, domain-less non-success log line is unactionable updater noise"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn updater_endpoint_non_success_anchor_does_not_silence_unrelated_errors() {
    // The new anchor is the literal plugin string. Other updater failures
    // that DO carry an actionable signal (signature/permission failures on
    // apply, deserialize errors) and unrelated non-updater errors that
    // merely mention a status code MUST NOT be dropped by it. Pin the
    // rejection contract so a future refactor doesn't loosen the substring.
    for msg in [
        "failed to apply update: signature verification failed",
        "failed to deserialize update response: missing field `version`",
        "backend request to /agent-integrations failed with status code 500",
        "tool exited with non-zero status code 1",
    ] {
        let event = event_with_tags_and_message(&[], msg);
        assert!(
            !is_updater_transient_event(&event),
            "unrelated/actionable error must still reach Sentry: {msg}"
        );
    }
}

#[test]
fn message_failure_classifier_matches_canonical_status_phrases() {
    for msg in [
        "rpc.invoke_method failed: GET /teams failed (502 Bad Gateway)",
        "GET /teams/me/usage failed (503 Service Unavailable)",
        "downstream returned (504 Gateway Timeout): retry budget exhausted",
        "OpenHuman API error (520 <unknown status code>): cf",
        "POST /channels/telegram/typing failed (429 Too Many Requests)",
        "auth connect failed: 503 Service Unavailable",
    ] {
        assert!(
            is_transient_message_failure(msg),
            "{msg:?} must be classified as transient"
        );
    }
}

#[test]
fn message_failure_classifier_matches_transport_phrases() {
    for msg in [
        "integrations.get failed: composio/tools → operation timed out",
        "GET https://api.example.com → connection forcibly closed (os 10054)",
        "POST /v1/foo → tls handshake eof",
        "error sending request for url (https://api.example.com)",
    ] {
        assert!(
            is_transient_message_failure(msg),
            "{msg:?} must be classified as transient"
        );
    }
}

#[test]
fn message_failure_classifier_keeps_unrelated_messages() {
    for msg in [
        "rpc.invoke_method failed: schema validation error",
        "process 502 exited unexpectedly",
        "GET /teams failed (404 Not Found)",
        "GET /teams failed (500 Internal Server Error)",
        "unrelated error with port 5023",
        "",
    ] {
        assert!(
            !is_transient_message_failure(msg),
            "{msg:?} must not be classified as transient"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn budget_filter_drops_budget_message_on_tagged_400() {
    let event = event_with_tags_and_message(
        &[("failure", "non_2xx"), ("status", "400")],
        r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Insufficient budget"}"#,
    );

    assert!(is_budget_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn budget_filter_drops_budget_exception_on_tagged_400() {
    let mut event = event_with_tags(&[("failure", "non_2xx"), ("status", "400")]);
    event.exception.values.push(sentry::protocol::Exception {
        value: Some("Budget exceeded — add credits to continue".to_string()),
        ..Default::default()
    });

    assert!(is_budget_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn budget_filter_keeps_non_budget_400() {
    let event = event_with_tags_and_message(
        &[("failure", "non_2xx"), ("status", "400")],
        "Bad request: missing field",
    );

    assert!(!is_budget_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn budget_filter_requires_non_2xx_failure_and_400_status() {
    let message = "Budget exceeded — add credits to continue";
    for tags in [
        vec![("failure", "transport"), ("status", "400")],
        vec![("failure", "non_2xx"), ("status", "500")],
        vec![("failure", "non_2xx")],
    ] {
        let event = event_with_tags_and_message(&tags, message);
        assert!(!is_budget_event(&event));
    }
}

#[test]
fn report_error_or_expected_does_not_panic() {
    report_error_or_expected(
        "local ai is disabled",
        "rpc",
        "invoke_method",
        &[("method", "openhuman.inference_prompt")],
    );
    report_error_or_expected(
        "ollama API key not set",
        "agent",
        "provider_chat",
        &[("provider", "ollama")],
    );
    // #2079 / #2076 / #2202 — exercises the expected_error_kind
    // ProviderConfigRejection branch AND the report_expected_message
    // skip-log arm (the agent/web-channel re-report demotion path).
    report_error_or_expected(
        "agent.run_single failed: custom_openai API error (400 Bad Request): \
         The supported API model names are deepseek-v4-pro or deepseek-v4-flash, \
         but you passed reasoning-v1.",
        "agent",
        "native_chat",
        &[("provider", "custom_openai")],
    );
    report_error_or_expected(
        "custom_openai API error (400): invalid temperature: only 1 is allowed for this model",
        "web_channel",
        "run_chat_task",
        &[("provider", "custom_openai")],
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn quota_exhausted_filter_matches_500_wrapped_kiro_event() {
    // TAURI-RUST-C9A: verbatim message as formatted by the provider emit
    // site — a 500 envelope around an inner 402 / MONTHLY_REQUEST_COUNT.
    // The status-agnostic quota filter must catch it on the message path
    // and the exception path.
    let body = "kiro API error (500 Internal Server Error): {\"error\":{\"message\":\
        \"HTTP 402 from Kiro IDE: {\\\"reason\\\":\\\"MONTHLY_REQUEST_COUNT\\\"}\",\
        \"type\":\"server_error\"}}";
    assert!(is_quota_exhausted_event(&event_with_message(body)));
    assert!(is_quota_exhausted_event(&event_with_exception_value(body)));
    assert!(is_quota_exhausted_message(body));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn quota_exhausted_filter_matches_responses_usage_limit_reached_event() {
    // TAURI-RUST-AFE: verbatim message as formatted by the `chat_via_responses`
    // emit site — the Codex/ChatGPT OAuth `/responses` plan cap. Mirrors the
    // real production shape `"<name> Responses API error (<status>): <body>"`
    // (`compatible_helpers.rs:135`), including the `(429)` status segment and
    // the full AFE payload (`plan_type` + `resets_at`) from `provider/http_error.rs`,
    // so this stays coupled to the actual wire format rather than a loose
    // substring. No "monthly"/"quota" co-marker, so it exercises the AFE
    // phrase extension reaching the before_send net on both message and
    // exception paths (a background retry loop waits until `resets_at`).
    let body = "openai Responses API error (429): {\"error\":{\"type\":\
        \"usage_limit_reached\",\"message\":\"The usage limit has been reached\",\
        \"plan_type\":\"plus\",\"resets_at\":1750000000}}";
    assert!(is_quota_exhausted_event(&event_with_message(body)));
    assert!(is_quota_exhausted_event(&event_with_exception_value(body)));
    assert!(is_quota_exhausted_message(body));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn quota_exhausted_filter_ignores_generic_500_and_rate_limit() {
    // A generic 500 outage and a 429 rate-limit are not plan-quota
    // exhaustion — they must keep reaching Sentry / their own handling.
    assert!(!is_quota_exhausted_event(&event_with_message(
        "kiro API error (500 Internal Server Error): upstream connection reset"
    )));
    assert!(!is_quota_exhausted_event(&event_with_message(
        "provider API error (429 Too Many Requests): rate_limit_exceeded"
    )));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn insufficient_credits_filter_matches_message_path() {
    // Verbatim TAURI-RUST-C62 message as formatted by the provider emit
    // sites: "<provider> API error (402 Payment Required): <body>".
    let event = event_with_message(
        "myopenrouter API error (402 Payment Required): This request requires more credits, \
         or fewer max_tokens. You requested up to 65536 tokens, but can only afford 49732.",
    );
    assert!(is_insufficient_credits_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn insufficient_credits_filter_matches_exception_path() {
    let event = event_with_exception_value(
        "myopenrouter API error (402 Payment Required): insufficient balance",
    );
    assert!(is_insufficient_credits_event(&event));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn insufficient_credits_filter_requires_both_402_and_credit_phrase() {
    // A 402 with no credit phrase must NOT be swallowed (could be another
    // payment semantic) ...
    assert!(!is_insufficient_credits_event(&event_with_message(
        "provider API error (402): some unrelated condition"
    )));
    // ... and a credit phrase without a 402 must NOT be swallowed (e.g. a
    // 400/500 that merely mentions balance) so a real defect still pages.
    assert!(!is_insufficient_credits_event(&event_with_message(
        "provider API error (500): internal error, insufficient memory"
    )));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn insufficient_credits_filter_ignores_402_digits_in_a_non_402_body() {
    // A non-402 error whose body merely contains the digits "402" and a
    // credit phrase must NOT be suppressed — the 402 must be the status,
    // not an arbitrary number in the body.
    assert!(!is_insufficient_credits_event(&event_with_message(
        "provider API error (400): can only afford 402 tokens"
    )));
}

#[test]
fn is_insufficient_credits_message_matches_verbatim_cron_402() {
    // Verbatim TAURI-RUST-514 body as it reaches the cron `report_error`
    // call site (`domain=cron`, `operation=agent_job`): the message-level
    // matcher must catch it so the cron halt skips the leaking report.
    assert!(is_insufficient_credits_message(
        "openrouter API error (402 Payment Required): {\"error\":{\"message\":\"This \
         request requires more credits, or fewer max_tokens. You requested up to 65536 \
         tokens, but can only afford 5081.\"}}",
    ));
    assert!(is_insufficient_credits_message(
        "custom_openai API error (402 Payment Required): insufficient balance",
    ));
}

#[test]
fn is_insufficient_credits_message_requires_402_and_credit_phrase() {
    // A 402 without a credit phrase, and a credit phrase without a 402
    // status, must both stay reportable (could be a real defect).
    assert!(!is_insufficient_credits_message(
        "provider API error (402): some unrelated condition"
    ));
    assert!(!is_insufficient_credits_message(
        "provider API error (500): internal error, insufficient memory"
    ));
    // The status must be the 402, not a digit in the body.
    assert!(!is_insufficient_credits_message(
        "provider API error (400): can only afford 402 tokens"
    ));
    // codex P2: the status prefix "(402 Payment Required)" itself contains
    // the phrase "payment required". An unrelated body behind a real 402
    // must NOT be classified as insufficient-credits (else the cron halt
    // would suppress a genuine 402 defect). The credit signal must live in
    // the BODY, not the formatted status prefix.
    assert!(!is_insufficient_credits_message(
        "provider API error (402 Payment Required): some unrelated condition"
    ));
    // But a body that literally carries the credit signal still matches,
    // even when the status prefix also says "Payment Required".
    assert!(is_insufficient_credits_message(
        "provider API error (402 Payment Required): your account has insufficient balance"
    ));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn is_insufficient_credits_event_delegates_to_message_matcher() {
    // Parity: the event-level filter is now a thin wrapper over the
    // message-level matcher across both the message and exception paths.
    let body = "myopenrouter API error (402 Payment Required): This request requires \
                more credits, or fewer max_tokens.";
    assert!(is_insufficient_credits_message(body));
    assert!(is_insufficient_credits_event(&event_with_message(body)));
    assert!(is_insufficient_credits_event(&event_with_exception_value(
        body
    )));
}

#[test]
fn ollama_cloud_internal_500_reraise_routes_through_expected_path() {
    // TAURI-RUST-5MV — the actionable message the emit sites raise must
    // demote to `TransientUpstreamHttp` when re-reported at the agent / RPC
    // boundary (`provider_chat` → `report_error_or_expected`), so the
    // `domain=agent` half of the flood is suppressed too.
    let reraise = tinyinference_llm::failure::ollama_cloud_internal_500_user_message(
        Some("minimax-m3:cloud"),
        500,
    );
    assert_eq!(
        expected_error_kind(&reraise),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn ollama_cloud_internal_500_before_send_matches_raw_and_reraised_shapes() {
    // The outermost net catches BOTH the raw emit body (any compatible
    // path that bypassed the cascade) and the actionable re-raise.
    let raw = "ollama API error (500 Internal Server Error): \
        {\"error\":\"Internal Server Error (ref: df512dcb-d915-493b-8f2d-e8d3dfa640c1)\"}";
    assert!(is_ollama_cloud_internal_500_event(&event_with_message(raw)));
    assert!(is_ollama_cloud_internal_500_event(
        &event_with_exception_value(raw)
    ));

    let reraise = tinyinference_llm::failure::ollama_cloud_internal_500_user_message(None, 500);
    assert!(is_ollama_cloud_internal_500_event(&event_with_message(
        &reraise
    )));

    // A local Ollama 500 without the `ref:` envelope, and a non-ollama 500,
    // both stay reportable.
    assert!(!is_ollama_cloud_internal_500_event(&event_with_message(
        "ollama API error (500 Internal Server Error): {\"error\":\"out of memory\"}"
    )));
    assert!(!is_ollama_cloud_internal_500_event(&event_with_message(
        "openai API error (500): Internal Server Error (ref: abc)"
    )));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn session_expired_before_send_matches_core_401_events() {
    let msg = "SESSION_EXPIRED: backend session not active — sign in to resume LLM work";
    for event in [
        event_with_tags_and_message(&[("domain", "llm_provider"), ("status", "401")], msg),
        {
            let mut event = event_with_exception_value(msg);
            event.tags.insert("domain".into(), "backend_api".into());
            event.tags.insert("status".into(), "401".into());
            event
        },
    ] {
        assert!(
            is_session_expired_event(&event),
            "core/backend session-expired 401 events should be filtered"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn session_expired_before_send_stays_domain_scoped() {
    let event = event_with_tags_and_message(
        &[("domain", "composio"), ("status", "401")],
        "SESSION_EXPIRED: backend session not active — sign in to resume LLM work",
    );
    assert!(
        !is_session_expired_event(&event),
        "non-core domains must not be filtered as backend session expiry"
    );
}
