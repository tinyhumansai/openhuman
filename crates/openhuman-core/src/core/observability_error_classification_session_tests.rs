use super::*;

#[test]
fn couples_list_models_404_source_shape_to_classifier() {
    // TAURI-RUST-8X3 coupling guard. Ties the TYPED SOURCE error shape
    // emitted by `inference/provider/ops/models.rs` (the
    // `provider returned 404: <body>` format) to the classifier, so a
    // wording / prefix drift fails CI instead of silently leaking events.
    //
    // The wild Sentry message carried the `inference/ops.rs`
    // `error!("[inference::ops] list_models:error: {err}")` PREFIX. The
    // primary fix classifies the raw `err` at the source before that
    // prefix is applied, but the `contains` widening above must ALSO
    // catch the prefixed variant for any future prefixed re-report path.
    // Assert BOTH the raw source shape and the prefixed log-line shape.

    // (a) Raw source shape — exactly what `models.rs` returns for a Go
    //     default-handler 404 (`404 page not found`).
    let raw_source = "provider returned 404: 404 page not found";
    assert_eq!(
        expected_error_kind(raw_source),
        Some(ExpectedErrorKind::ProviderUserState),
        "raw list_models 404 source shape must classify as ProviderUserState"
    );

    // (b) Raw source shape WITH the actionable hint appended by
    //     `models.rs` for the 404 case — the prefix anchor must survive
    //     the suffix.
    let raw_with_hint = "provider returned 404: 404 page not found — the configured base URL does not expose a `/models` endpoint; check the provider's base URL (it usually ends in `/v1`)";
    assert_eq!(
        expected_error_kind(raw_with_hint),
        Some(ExpectedErrorKind::ProviderUserState),
        "list_models 404 + actionable hint must still classify as ProviderUserState"
    );

    // (c) Prefixed log-line shape — the exact pattern from
    //     `inference/ops.rs::inference_list_models` `error!`. The explicit
    //     `list_models:error: provider returned 404` anchor must catch this
    //     even though it does not start with `provider returned 404`. The
    //     anchor is the formatted prefix, not a bare `404` substring, so it
    //     does NOT mis-fire on a 500 whose body merely relays an upstream
    //     404 (see `does_not_classify_non_404_list_models_failures_as_user_state`).
    let prefixed = "[inference::ops] list_models:error: provider returned 404: 404 page not found";
    assert_eq!(
        expected_error_kind(prefixed),
        Some(ExpectedErrorKind::ProviderUserState),
        "prefixed list_models 404 log line must still classify as ProviderUserState (anchored prefix)"
    );
}

#[test]
fn does_not_classify_non_404_list_models_failures_as_user_state() {
    // Discrimination guard: only the 404 prefix demotes. Sibling 4xx /
    // 5xx codes from the same `provider returned NNN:` emit site must
    // stay actionable in Sentry — they map to BYO-key auth walls (401 /
    // 403), client-shape bugs (400), and transient / server faults
    // (429 / 5xx) respectively. Pinning each shape here protects the
    // #2286 BYO-key 401 contract and prevents the arm from silently
    // widening to all 4xx.
    for raw in [
        // BYO-key auth wall — must still escalate (`does_not_classify_byo_key_provider_401_as_session_expired` sibling guard).
        r#"provider returned 401: {"error":"Invalid API key"}"#,
        r#"provider returned 403: {"error":"Forbidden: API key revoked"}"#,
        // Request-shape mismatch — likely a bug in our client.
        r#"provider returned 400: {"error":"Bad Request"}"#,
        // Transient — caught by retry/backoff at the provider layer,
        // does NOT belong in the user-state bucket.
        r#"provider returned 429: {"error":"rate_limited"}"#,
        r#"provider returned 503: upstream temporarily unavailable"#,
        // 500 — a real upstream bug; must reach Sentry.
        r#"provider returned 500: {"error":"internal_server_error"}"#,
        // TAURI-RUST-8X3 false-negative guard: a genuine 4xx/5xx whose
        // *body* merely relays an upstream 404 phrase. The actual status
        // is 500/400, so this is a real failure that MUST reach Sentry —
        // the classifier anchors on the `provider returned 404:` prefix,
        // not on any `404` occurrence in the body, so these must NOT demote.
        r#"provider returned 500: {"error":"upstream provider returned 404 page not found"}"#,
        r#"provider returned 400: gateway error — upstream provider returned 404"#,
        // Prefixed log-line variant of the same trap: the genuine status is
        // 500, the buried `... 404` is body text.
        "[inference::ops] list_models:error: provider returned 500: upstream provider returned 404 not found",
    ] {
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "non-404 list_models failure must NOT demote to ProviderUserState: {raw}"
        );
    }
}

#[test]
fn classifies_local_ai_binary_missing_errors() {
    // OPENHUMAN-TAURI-9N: `local_ai_tts` returns this exact string
    // from `service::speech::tts` when piper isn't on PATH or
    // `PIPER_BIN` isn't set.
    assert_eq!(
        expected_error_kind("piper binary not found. Set PIPER_BIN or install piper."),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
    // Sibling shapes from the same service area share the anchor and
    // must classify the same way — the user-facing remediation is
    // identical (install / configure the binary).
    assert_eq!(
        expected_error_kind(
            "Ollama binary not found at '/usr/local/bin/ollama'. Provide a valid path to the ollama executable."
        ),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
    assert_eq!(
        expected_error_kind("Ollama installed but binary not found on system"),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
    // Wrapped by the RPC dispatcher in production:
    //   `"rpc.invoke_method failed: piper binary not found. …"`.
    // The classifier is substring-based, so caller context must not
    // defeat it.
    assert_eq!(
        expected_error_kind(
            "rpc.invoke_method failed: piper binary not found. Set PIPER_BIN or install piper."
        ),
        Some(ExpectedErrorKind::LocalAiBinaryMissing)
    );
}

#[test]
fn does_not_classify_unrelated_messages_as_binary_missing() {
    // Pin the anchor: messages that talk about binaries in a
    // different context (download failures, version mismatches)
    // must not be silenced.
    assert_eq!(
        expected_error_kind("piper binary failed to spawn: permission denied"),
        None
    );
    assert_eq!(
        expected_error_kind("piper returned an empty audio file"),
        None
    );
}

#[test]
fn classifies_session_expired_messages() {
    // OPENHUMAN-TAURI-26: the canonical wire shape that `agent.run_single`
    // and `web_channel.run_chat_task` re-emit via `report_error_or_expected`
    // when the user's JWT expires mid-conversation. The classifier
    // anchors on the literal `"session expired"` substring from the
    // OpenHuman backend's 401 body — NOT on the bare `(401 Unauthorized)`
    // status, which would also silence BYO-key OpenAI/Anthropic 401s
    // that are actionable.
    assert_eq!(
        expected_error_kind(
            r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Session expired. Please log in again."}"#
        ),
        Some(ExpectedErrorKind::SessionExpired)
    );

    // Wrapped by the agent / web-channel report sites in production —
    // the classifier is substring-based so caller context must not
    // defeat it.
    assert_eq!(
        expected_error_kind(
            r#"run_chat_task failed client_id=abc thread_id=t1 request_id=r1 error=OpenHuman API error (401 Unauthorized): {"success":false,"error":"Session expired. Please log in again."}"#
        ),
        Some(ExpectedErrorKind::SessionExpired)
    );

    // Sentinel raised by `providers::openhuman_backend::resolve_bearer`
    // when the scheduler-gate signed-out override is set
    // (OPENHUMAN-TAURI-1T's cascade dampener returns this so callers
    // get the same teardown path as a real backend 401).
    assert_eq!(
        expected_error_kind(
            "SESSION_EXPIRED: backend session not active — sign in to resume LLM work"
        ),
        Some(ExpectedErrorKind::SessionExpired)
    );

    // Local pre-flight guards — OpenHuman-specific phrasing, safe to
    // match regardless of caller wrapping.
    for raw in [
        "no backend session token; run auth_store_session first",
        "session JWT required",
        "composio unavailable: no backend session token. Sign in first (auth_store_session).",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::SessionExpired),
            "should classify as session-expired: {raw}"
        );
    }
}

/// OPENHUMAN-TAURI-SG (33 events, escalating, release `0.53.43+2b64ea8…`):
/// pre-#1763 leak of the `resolve_bearer` sentinel through
/// `agent.run_single`. PR #1763 (1fb0bef5) wired the `SessionExpired`
/// arm and the existing `classifies_session_expired_messages` test
/// covers the same byte string — this test pins the *Sentry-event
/// verbatim* shape (taken from the OPENHUMAN-TAURI-SG event payload)
/// so a future tweak to `is_session_expired_message` cannot regress
/// this exact wire form without a red test.

#[test]
fn session_expired_sg_wire_shape_matches() {
    let msg = "SESSION_EXPIRED: backend session not active — sign in to resume LLM work";
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "OPENHUMAN-TAURI-SG wire shape must classify as SessionExpired — \
         a regression here re-leaks 33+ events/cycle to Sentry"
    );
}

/// The two sibling `SESSION_EXPIRED:` bail sites in
/// `providers::factory::verify_session_active` emit different message
/// suffixes but the same sentinel prefix. They route through the same
/// classifier as the run_single bail at
/// `providers::openhuman_backend::resolve_bearer`, and any matcher
/// tweak that breaks the family (e.g. moving from `contains` to a
/// stricter prefix/suffix match) would re-leak ALL of them. Pin every
/// variant the codebase actually emits so a future regression on the
/// matcher is caught for the whole family, not just the SG instance.

#[test]
fn session_expired_sibling_family_factory_strings_match() {
    // crates/openhuman-core/src/inference/provider/factory.rs:247
    // (verify_session_active — scheduler_gate signed-out path)
    let custom_providers_variant =
        "SESSION_EXPIRED: backend session not active — sign in to use custom providers";
    // crates/openhuman-core/src/inference/provider/factory.rs:266
    // (verify_session_active — empty auth-profile JWT path)
    let no_backend_session_variant =
        "SESSION_EXPIRED: no backend session — sign in to use OpenHuman";

    for raw in [custom_providers_variant, no_backend_session_variant] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::SessionExpired),
            "factory.rs sibling sentinel must classify as SessionExpired: {raw}"
        );
    }
}

/// OPENHUMAN-TAURI-4P0: the OpenHuman backend rejects an expired/
/// revoked JWT with the envelope `{"success":false,"error":"Invalid
/// token"}` (vs. the explicit `"Session expired. Please log in again."`
/// body covered by `classifies_session_expired_messages`). Same emit
/// site, same wrapping by `web_channel.run_chat_task`, but the body
/// substring is different.
///
/// The matcher uses a conjunctive `"OpenHuman API error (401"` +
/// envelope-shaped `"\"error\":\"Invalid token\""` anchor pair so the
/// #2286 contract for bare `"Invalid token"` / BYO-key 401s is
/// preserved — `does_not_classify_byo_key_provider_401_as_session_expired`
/// pins that and must stay green.

#[test]
fn classifies_openhuman_invalid_token_401_as_session_expired() {
    // Verbatim wire shape from the OPENHUMAN-TAURI-4P0 event payload.
    let msg = r#"run_chat_task failed client_id=lssXhQidBfzGXG9k thread_id=thread-743193ba-f0c1-4008-b665-64d3030d1453 request_id=00696b71-fa05-4574-bcdb-5744a5dac6ea error=OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "OPENHUMAN-TAURI-4P0 verbatim wire shape must classify as SessionExpired"
    );

    // Unwrapped emit shape (without the run_chat_task prefix) — also
    // appears at provider/agent layers; the substring matcher must
    // catch it regardless of caller wrapping.
    assert_eq!(
        expected_error_kind(
            r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#
        ),
        Some(ExpectedErrorKind::SessionExpired),
        "unwrapped OpenHuman invalid-token envelope must classify as SessionExpired"
    );
}

/// TAURI-RUST-4K5 (118 events, escalating on 0.56.0): the embedding
/// client in `tinyinference-embeddings/src/cloud.rs` wraps the same
/// OpenHuman backend `{"success":false,"error":"Invalid token"}` 401
/// envelope as 4P0, but with the `"Embedding API error"` prefix
/// instead of `"OpenHuman API error"` (different emit-site format
/// string, same underlying session-expired cause — see breadcrumb
/// `[scheduler_gate] signed_out false -> true` immediately preceding
/// the 401 in the event payload).
///
/// Uses the same conjunctive `"<prefix> (401"` + envelope-shaped
/// `"\"error\":\"Invalid token\""` anchor pattern as 4P0 so the
/// #2286 / BYO-key contract is preserved — covered by
/// `does_not_classify_byo_key_provider_401_as_session_expired` and
/// `does_not_classify_embedding_byo_key_401_as_session_expired`
/// (below).

#[test]
fn classifies_embedding_api_invalid_token_401_as_session_expired() {
    // Verbatim wire shape from the TAURI-RUST-4K5 event payload (Sentry
    // issue 5230, latest event 2026-05-27 20:49 on openhuman@0.56.0,
    // domain=embeddings operation=openai_embed status=401).
    let msg =
        r#"Embedding API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "TAURI-RUST-4K5 verbatim wire shape must classify as SessionExpired"
    );

    // The substring matcher must survive caller wrapping the same way
    // the 4P0 web-channel `run_chat_task` test wraps the body — callers
    // that re-emit through a tracing field or another layer prepend
    // arbitrary context.
    let wrapped = r#"openai_embed failed error=Embedding API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(wrapped),
        Some(ExpectedErrorKind::SessionExpired),
        "wrapped 4K5 envelope must still classify as SessionExpired"
    );
}

/// TAURI-RUST-1EE (Sentry issue 1807, 110 events, 109 on
/// openhuman@0.56.0): the streaming-chat path wraps the same OpenHuman
/// backend `{"success":false,"error":"Invalid token"}` 401 envelope
/// with the `"OpenHuman streaming API error"` prefix (emitted at
/// `inference/provider/compatible.rs:949`) — distinct from the
/// non-streaming `"OpenHuman API error"` prefix (4P0) and the
/// `"Embedding API error"` prefix (4K5). The `streaming` token between
/// `OpenHuman` and `API error` means the 4P0 anchor
/// (`"OpenHuman API error (401"`) does not match it, so it needs its
/// own prefix arm.

#[test]
fn classifies_openhuman_streaming_invalid_token_401_as_session_expired() {
    // Verbatim wire shape from the TAURI-RUST-1EE event payload
    // (domain=llm_provider operation=streaming_chat status=401
    // provider=OpenHuman model=reasoning-v1).
    let msg = r#"OpenHuman streaming API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(msg),
        Some(ExpectedErrorKind::SessionExpired),
        "TAURI-RUST-1EE verbatim streaming wire shape must classify as SessionExpired"
    );

    // Caller-wrapped (agent.run_single / web_channel.run_chat_task
    // re-emit prepends context) must still classify.
    let wrapped = r#"run_chat_task failed error=OpenHuman streaming API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert_eq!(
        expected_error_kind(wrapped),
        Some(ExpectedErrorKind::SessionExpired),
        "wrapped 1EE streaming envelope must still classify as SessionExpired"
    );
}

/// Polarity guard for the 1EE streaming arm — a third-party BYO-key
/// provider's streaming 401 (`"OpenAI streaming API error (401 …):
/// invalid_api_key"`) must STILL reach Sentry as actionable
/// misconfiguration. The `"OpenHuman streaming API error (401"` prefix
/// gate keeps the match OpenHuman-scoped.

#[test]
fn does_not_classify_streaming_byo_key_401_as_session_expired() {
    for raw in [
        "OpenAI streaming API error (401 Unauthorized): invalid_api_key",
        r#"OpenAI streaming API error (401 Unauthorized): {"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
        "Anthropic streaming API error (401): authentication_error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "BYO-key streaming 401 must reach Sentry as actionable error: {raw}"
        );
    }
}

/// Polarity guard for the 4K5 arm. The classifier must NOT swallow
/// `"Embedding API error (401 …)"` shapes from third-party BYO-key
/// embedding providers (OpenAI / Voyage / Cohere upstream rejecting
/// the user's own API key). Those are actionable user-config errors
/// that need to reach Sentry — same contract as
/// `does_not_classify_byo_key_provider_401_as_session_expired` for
/// the OpenAI chat API.

#[test]
fn does_not_classify_embedding_byo_key_401_as_session_expired() {
    for raw in [
        "Embedding API error (401 Unauthorized): invalid_api_key",
        r#"Embedding API error (401 Unauthorized): {"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
        // Wire shape without the OpenHuman envelope — bare provider
        // rejection prose. Must reach Sentry as actionable BYO-key
        // misconfiguration.
        "Embedding API error (401): authentication_error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "BYO-key embedding 401 must reach Sentry as actionable error: {raw}"
        );
    }
}

#[test]
fn does_not_classify_byo_key_provider_401_as_session_expired() {
    // Critical: a BYO-key 401 from OpenAI / Anthropic etc. is an
    // actionable misconfiguration (wrong API key) that the user needs
    // to fix in settings. It must reach Sentry as an error and must
    // NOT be classified as session-expired at the agent layer — the
    // strict classifier requires the OpenHuman backend's
    // "session expired" body to anchor the match. The JSON-RPC
    // dispatch-site classifier uses the same strict rule so these
    // scoped provider failures never clear the app session either.
    for raw in [
        "OpenAI API error (401 Unauthorized): invalid_api_key",
        "Anthropic API error (401 Unauthorized): authentication_error",
        "OpenAI API error (401): unauthorized",
        r#"OpenAI API error (401 Unauthorized): {"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
        // Generic "invalid token" without OpenHuman session phrasing —
        // could mean a third-party provider rejected its own token.
        "Invalid token",
        "got an invalid token here",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "BYO-key / generic 401 must reach Sentry as actionable error: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_messages_as_session_expired() {
    // Bare numeric 401 (port number, runbook reference) must not be
    // silenced.
    assert_eq!(expected_error_kind("server returned 401"), None);
    assert_eq!(
        expected_error_kind("see runbook for 401 handling at https://example.com/401"),
        None
    );
    // Provider 5xx — must reach Sentry.
    assert_eq!(
        expected_error_kind("OpenAI API error (500): internal server error"),
        None
    );
    // Lowercase sentinel must NOT match — the SESSION_EXPIRED sentinel
    // is case-sensitive by design (matches the sentinel emitted by
    // `providers::openhuman_backend::resolve_bearer` exactly).
    assert_eq!(expected_error_kind("session_expired lowercase"), None);
}

#[test]
fn report_error_does_not_panic_with_many_tags() {
    let err = anyhow::anyhow!("multi-tag");
    report_error(
        &err,
        "test",
        "multi_tag",
        &[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")],
    );
}

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
    // `API_KEY_REJECTED:` sentinel (`api::rest::flatten_authed_error`). The
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
