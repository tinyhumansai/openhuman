#[cfg(feature = "crash-reporting")]
#[test]
fn transient_filter_drops_429_408_502_503_504() {
    for status in ["429", "408", "502", "503", "504"] {
        let event = event_with_tags(&[
            ("domain", "llm_provider"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_provider_http_failure(&event),
            "status {status} must be classified as transient and filtered"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn custom_openai_502_event_shape_is_transient_provider_http() {
    let event = event_with_tags_and_message(
        &[
            ("domain", "llm_provider"),
            ("provider", "custom_openai"),
            ("failure", "non_2xx"),
            ("status", "502"),
        ],
        "custom_openai API error (502 Bad Gateway): upstream gateway blip",
    );
    assert!(
        is_transient_provider_http_failure(&event),
        "custom_openai 502 attempts should be treated as transient provider HTTP noise"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_filter_keeps_permanent_failures() {
    for status in ["400", "401", "403", "404", "500"] {
        let event = event_with_tags(&[
            ("domain", "llm_provider"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_provider_http_failure(&event),
            "status {status} must NOT be filtered — it's actionable"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_filter_keeps_aggregate_all_exhausted() {
    let event = event_with_tags(&[
        ("domain", "llm_provider"),
        ("failure", "all_exhausted"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "aggregate all_exhausted events must surface (they are the cascade signal)"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn all_exhausted_aggregate_is_dropped_only_when_every_attempt_is_transient() {
    let tags = [
        ("domain", "llm_provider"),
        ("failure", "all_exhausted"),
        ("attempts", "2"),
    ];
    let transient = event_with_tags_and_message(
        &tags,
        "All providers/models failed. Attempts: openai API error (503 Service Unavailable); custom_openai API error (502 Bad Gateway)",
    );
    assert!(
        is_all_transient_provider_exhaustion_event(&transient),
        "all-transient aggregate should not recreate per-attempt Sentry noise"
    );
    let mixed = event_with_tags_and_message(
        &tags,
        "All providers/models failed. Attempts: openai API error (401 Unauthorized); custom_openai API error (503 Service Unavailable)",
    );
    assert!(
        !is_all_transient_provider_exhaustion_event(&mixed),
        "mixed/permanent aggregate should remain actionable"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_filter_keeps_events_with_no_status_tag() {
    let event = event_with_tags(&[("domain", "llm_provider"), ("failure", "non_2xx")]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "missing status tag must not be silently dropped"
    );
}

// Regression guard: the filter must scope to provider events only. Other
// subsystems emit `failure=non_2xx` (e.g.
// `providers/compatible.rs` uses the same marker for OAI-compatible
// error paths, but every site goes through `report_error(..,
// "llm_provider", ..)` so the domain tag is consistent), but the broader
// point is: any future caller that re-uses the same tag set for a
// different domain must NOT be silently dropped by this filter.

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_filter_keeps_events_with_no_domain_tag() {
    let event = event_with_tags(&[("failure", "non_2xx"), ("status", "503")]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "missing domain tag means the event isn't provider-originated — must surface"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn transient_filter_keeps_events_from_other_domains() {
    let event = event_with_tags(&[
        ("domain", "scheduler"),
        ("failure", "non_2xx"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_provider_http_failure(&event),
        "non-provider domain must surface even if failure/status tags collide"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn backend_api_filter_drops_transient_statuses() {
    for status in TRANSIENT_HTTP_STATUSES {
        let event = event_with_tags(&[
            ("domain", "backend_api"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_transient_backend_api_failure(&event),
            "backend status {status} must be classified as transient"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn backend_api_filter_drops_transient_transport_phrases() {
    for phrase in TRANSIENT_TRANSPORT_PHRASES {
        let event = event_with_tags_and_message(
            &[("domain", "backend_api"), ("failure", "transport")],
            &format!("GET /teams failed: {phrase}"),
        );
        assert!(
            is_transient_backend_api_failure(&event),
            "backend transport phrase {phrase} must be classified as transient"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn backend_api_filter_keeps_non_transient_failures() {
    for status in ["404", "500"] {
        let event = event_with_tags(&[
            ("domain", "backend_api"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_transient_backend_api_failure(&event),
            "backend status {status} must stay visible"
        );
    }

    let wrong_domain = event_with_tags(&[
        ("domain", "scheduler"),
        ("failure", "non_2xx"),
        ("status", "503"),
    ]);
    assert!(
        !is_transient_backend_api_failure(&wrong_domain),
        "domain scoping must keep unrelated transient-shaped events visible"
    );

    let non_matching_transport = event_with_tags_and_message(
        &[("domain", "backend_api"), ("failure", "transport")],
        "GET /teams failed: certificate verify failed",
    );
    assert!(
        !is_transient_backend_api_failure(&non_matching_transport),
        "transport failures without an allowlisted phrase must stay visible"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn skills_install_fetch_filter_drops_client_error_statuses() {
    for status in ["400", "401", "403", "404", "410", "499"] {
        let event = event_with_tags(&[
            ("domain", "skills"),
            ("operation", "install_fetch"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            is_skill_install_user_fetch_failure(&event),
            "skills install_fetch status {status} must be treated as user/catalog state"
        );
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn skills_install_fetch_filter_keeps_server_and_wrong_shape_failures() {
    for status in ["500", "502", "503"] {
        let event = event_with_tags(&[
            ("domain", "skills"),
            ("operation", "install_fetch"),
            ("failure", "non_2xx"),
            ("status", status),
        ]);
        assert!(
            !is_skill_install_user_fetch_failure(&event),
            "skills install_fetch status {status} must remain reportable"
        );
    }

    for tags in [
        [
            ("domain", "skills"),
            ("operation", "install_fetch"),
            ("failure", "transport"),
            ("status", "404"),
        ],
        [
            ("domain", "skills"),
            ("operation", "run"),
            ("failure", "non_2xx"),
            ("status", "404"),
        ],
        [
            ("domain", "backend_api"),
            ("operation", "install_fetch"),
            ("failure", "non_2xx"),
            ("status", "404"),
        ],
    ] {
        let event = event_with_tags(&tags);
        assert!(
            !is_skill_install_user_fetch_failure(&event),
            "only skills.install_fetch non_2xx 4xx events may be filtered: {tags:?}"
        );
    }
}

#[test]
fn classifies_api_key_rejected_as_expected_credential_lapse() {
    // A 401 on a TinyHumans API-key credential flattens to the
    // `API_KEY_REJECTED:` sentinel (`backend::client::flatten_authed_error`). The
    // remedy is a new key, so it must not reach Sentry as an RPC error.
    let msg = format!("{API_KEY_REJECTED_PREFIX} backend rejected api key on GET /teams/me/usage");
    assert!(is_api_key_rejected_message(&msg));
    assert!(matches!(
        expected_error_kind(&msg),
        Some(ExpectedErrorKind::SessionExpired)
    ));
    // …but it is not a session expiry: the JSON-RPC publish boundary keys
    // off `is_session_expired_message`, which must stay false so a bad key
    // never clears a signed-in session.
    assert!(!is_session_expired_message(&msg));
}

#[test]
fn classifies_offline_local_session_refusal_as_backend_unavailable() {
    // Sentry 36649 — 5.5k events: hosted RPCs (`team_get_usage`,
    // `announcements_get_latest`, `billing_*`) invoked under the offline
    // local credential. The refusal must classify as expected.
    let msg = crate::security::credentials::session_support::LOCAL_SESSION_BACKEND_UNAVAILABLE;
    assert!(matches!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::BackendUnavailable)
    ));
    assert!(!is_session_expired_message(msg));
}

#[test]
fn memory_plan_refusal_is_backend_user_state_not_session_expiry() {
    // #8081: memory not in the account's plan is user state for Sentry, and
    // must not read as an expired session (which signs the user out).
    let plan =
        "unauthorized: [MEMORY_PLAN_REQUIRED] memory API memory/scopes on api.tinyhumans.ai: \
                the session expired or the API key was rejected (HTTP 403 Forbidden); \
                re-authenticate — memory is not included in this account's plan";
    assert!(is_plan_required_message(plan));
    assert!(!is_session_expired_message(plan));
    assert_eq!(
        expected_error_kind(plan),
        Some(ExpectedErrorKind::BackendUserError)
    );
    assert!(!is_plan_required_message(
        "[USER_INSUFFICIENT_CREDITS] top up"
    ));
    // Only the delimited code in a hosted failure head counts, not a mention.
    assert!(!is_plan_required_message(
        "backend returned 500: unexpected [MEMORY_PLAN_REQUIRED] token"
    ));
    assert!(!is_plan_required_message(
        "engine error: saw [not a code_PLAN_REQUIRED] memory API x"
    ));
}
