use super::*;
use crate::core::events::DomainEvent;
use reqwest::StatusCode;

fn response(status: StatusCode, body: &str) -> reqwest::Response {
    reqwest::Response::from(
        axum::http::Response::builder()
            .status(status)
            .body(body.to_string())
            .expect("build response"),
    )
}

#[tokio::test]
async fn a_provider_error_body_is_read_only_up_to_the_cap() {
    let huge = "x".repeat(MAX_ERROR_BODY_BYTES * 4);
    let body = read_error_body(response(StatusCode::INTERNAL_SERVER_ERROR, &huge)).await;
    assert_eq!(body.len(), MAX_ERROR_BODY_BYTES);

    let small = read_error_body(response(StatusCode::BAD_REQUEST, "short body")).await;
    assert_eq!(small, "short body");
}

const FIXTURE_BACKEND: &str = "fixture-managed-provider";

fn install_fixture_backend_classifier() {
    use crate::inference::registry::{
        register_backend_error_classifier, BackendErrorClassification,
    };
    register_backend_error_classifier(std::sync::Arc::new(|provider, _message| {
        BackendErrorClassification {
            provider_owned: provider == Some(FIXTURE_BACKEND),
            backend_owned: false,
            reportable: false,
            token: None,
            kind: None,
            malformed_bad_request: false,
        }
    }));
}

/// Drain the shared bus until `pick` matches an event or the queue is empty.
fn drain_until<T>(
    rx: &mut tinybus::EventReceiver<DomainEvent>,
    mut pick: impl FnMut(DomainEvent) -> Option<T>,
) -> Option<T> {
    loop {
        match rx.try_recv() {
            Ok(event) => {
                if let Some(found) = pick(event) {
                    return Some(found);
                }
            }
            Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => return None,
        }
    }
}

#[test]
fn transient_statuses_are_not_reported_but_client_errors_are() {
    for s in [408u16, 429, 502, 503, 504, 520] {
        assert!(
            !should_report_provider_http_failure(StatusCode::from_u16(s).unwrap()),
            "{s} is transient"
        );
    }
    for s in [400u16, 401, 404, 500] {
        assert!(
            should_report_provider_http_failure(StatusCode::from_u16(s).unwrap()),
            "{s} is reportable"
        );
    }
}

#[tokio::test]
async fn api_error_replaces_ollama_cloud_500_body_with_guidance() {
    let err = api_error(
        "ollama",
        response(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"Internal Server Error (ref: 1b2c3d4e-0000-4000-8000-000000000000)"}"#,
        ),
    )
    .await;
    assert!(tinyinference_llm::failure::is_ollama_cloud_internal_500_message(&err.to_string()));
}

#[tokio::test]
async fn api_error_replaces_no_model_loaded_body_with_guidance() {
    let err = api_error(
        "lm_studio",
        response(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"No models loaded. Please load a model in the developer page or via `lms load`."}}"#,
        ),
    )
    .await;
    assert!(err
        .to_string()
        .to_ascii_lowercase()
        .contains("load a model"));
}

#[tokio::test]
async fn api_error_keeps_raw_message_for_unclassified_status() {
    let err = api_error("custom", response(StatusCode::NOT_FOUND, "nope")).await;
    assert!(err.to_string().contains("custom API error (404"));
}

#[tokio::test]
async fn backend_401_publishes_session_expired_through_reporter() {
    install_fixture_backend_classifier();
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().expect("bus").receiver();
    let marker = "REPORTER_MARKER_SESSION MONTHLY_REQUEST_COUNT quota exceeded";
    let _ = api_error(FIXTURE_BACKEND, response(StatusCode::UNAUTHORIZED, marker)).await;
    let seen = drain_until(&mut rx, |e| match e {
        DomainEvent::SessionExpired { reason, .. } if reason.contains(marker) => Some(()),
        _ => None,
    });
    assert!(seen.is_some(), "backend 401 must publish SessionExpired");
}

#[tokio::test]
async fn backend_403_does_not_publish_session_expired() {
    install_fixture_backend_classifier();
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().expect("bus").receiver();
    let marker = "REPORTER_MARKER_FORBIDDEN a paid plan is required";
    let _ = api_error(FIXTURE_BACKEND, response(StatusCode::FORBIDDEN, marker)).await;
    let seen = drain_until(&mut rx, |e| match e {
        DomainEvent::SessionExpired { reason, .. } if reason.contains(marker) => Some(()),
        _ => None,
    });
    assert!(
        seen.is_none(),
        "a backend 403 is a refusal, not an expired session"
    );
}

#[tokio::test]
async fn byo_401_publishes_api_key_rejected_not_session_expired() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().expect("bus").receiver();
    let provider = "reporter_byo_provider";
    let _ = api_error(
        provider,
        response(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"Invalid or missing API key","type":"authentication_error"}}"#,
        ),
    )
    .await;
    // The bus is process-wide: only a SessionExpired raised by this call (its
    // body is in the reason) counts, not one from a concurrent test. Drain
    // every queued event so an expiry published after the rejection is seen.
    let (mut session_expired, mut rejected) = (false, false);
    drain_until(&mut rx, |e| -> Option<()> {
        match e {
            DomainEvent::SessionExpired { reason, .. }
                if reason.contains("Invalid or missing API key") || reason.contains(provider) =>
            {
                session_expired = true
            }
            DomainEvent::ProviderApiKeyRejected { provider: p, .. } if p == provider => {
                rejected = true
            }
            _ => {}
        }
        None
    });
    assert!(rejected, "BYO key rejection must be surfaced");
    assert!(
        !session_expired,
        "a BYO key's 401 must not expire the OpenHuman session"
    );
}

#[tokio::test]
async fn backend_provider_does_not_get_byo_key_rejection() {
    install_fixture_backend_classifier();
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().expect("bus").receiver();
    let _ = api_error(
        FIXTURE_BACKEND,
        response(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"type":"authentication_error"}}"#,
        ),
    )
    .await;
    let rejected = drain_until(&mut rx, |e| match e {
        DomainEvent::ProviderApiKeyRejected { provider, .. } if provider == FIXTURE_BACKEND => {
            Some(())
        }
        _ => None,
    });
    assert!(
        rejected.is_none(),
        "backend 401 is session expiry, not a BYO key"
    );
}

/// Sentry). Getting this wrong in either direction is a regression:
/// over-matching silences real misconfig; under-matching is TAURI-RUST-N.
#[test]
fn is_backend_auth_failure_only_matches_openhuman_backend_401() {
    use reqwest::StatusCode;
    install_fixture_backend_classifier();
    let backend = FIXTURE_BACKEND;

    assert!(is_backend_auth_failure(backend, StatusCode::UNAUTHORIZED));

    // A 403 is a refusal (plan, tier, WAF) and the rest are server faults or
    // transient: none of them expires the session.
    for s in [
        StatusCode::FORBIDDEN,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::BAD_REQUEST,
        StatusCode::NOT_FOUND,
    ] {
        assert!(
            !is_backend_auth_failure(backend, s),
            "backend {s} must not be treated as session-expiry"
        );
    }

    // Third-party BYO-key 401/403 (user's own key revoked) must NOT be
    // silenced — that is actionable misconfiguration for Sentry.
    for provider in ["custom_openai", "OpenAI", "Anthropic", "openrouter"] {
        assert!(
            !is_backend_auth_failure(provider, StatusCode::UNAUTHORIZED),
            "{provider} 401 must reach Sentry as actionable BYO-key error"
        );
        assert!(
            !is_backend_auth_failure(provider, StatusCode::FORBIDDEN),
            "{provider} 403 must reach Sentry as actionable BYO-key error"
        );
    }
}

/// End-to-end through `api_error`: a non-backend 401 with an auth-error body
/// returns the sanitized provider error (so the chat/UI surface is unchanged)
/// while the BYO-auth branch demotes it from Sentry. Exercises the wired-in
/// cascade, not just the predicate in isolation.
#[tokio::test]
async fn api_error_byo_auth_failure_returns_message_via_demoted_branch() {
    let http_response = axum::http::Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(
            r#"{"error":{"message":"Invalid or missing API key","type":"authentication_error"}}"#
                .to_string(),
        )
        .expect("build 401 response");
    let response = reqwest::Response::from(http_response);

    let err = api_error("kiro", response).await;
    let msg = err.to_string();
    assert!(
        msg.contains("kiro API error (401"),
        "error must still carry the provider/status prefix for the UI: {msg}"
    );
    assert!(
        msg.to_ascii_lowercase()
            .contains("invalid or missing api key"),
        "sanitized upstream body must propagate to the caller: {msg}"
    );
}

/// End-to-end through `api_error`: a 500-wrapped monthly-quota refusal (the
/// Kiro IDE proxy nests its 402 / `MONTHLY_REQUEST_COUNT` inside a 500
/// envelope, TAURI-RUST-C9A) returns the sanitized provider error to the UI
/// while routing through the quota-exhausted demote branch — *before* the
/// `should_report_provider_http_failure(500)` status gate that would otherwise
/// page once per memory-extraction retry.
#[tokio::test]
async fn api_error_monthly_quota_returns_message_via_demoted_branch() {
    let body = "{\"error\":{\"message\":\"HTTP 402 from Kiro IDE: \
        {\\\"message\\\":\\\"You have reached the limit.\\\",\
        \\\"reason\\\":\\\"MONTHLY_REQUEST_COUNT\\\"}\",\"type\":\"server_error\"}}";
    let http_response = axum::http::Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(body.to_string())
        .expect("build 500 response");
    let response = reqwest::Response::from(http_response);

    let err = api_error("kiro", response).await;
    let msg = err.to_string();
    assert!(
        msg.contains("kiro API error (500"),
        "error must still carry the provider/status prefix for the UI: {msg}"
    );
    assert!(
        msg.contains("MONTHLY_REQUEST_COUNT"),
        "sanitized upstream quota body must propagate to the caller: {msg}"
    );
    // The body must classify as quota-exhausted so the demote branch — not the
    // 500 status gate — handles it.
    assert!(tinyinference_llm::failure::body_indicates_quota_exhausted(
        body
    ));
    assert!(should_report_provider_http_failure(
        StatusCode::INTERNAL_SERVER_ERROR
    ));
}

/// `publish_backend_session_expired` must emit a `SessionExpired` event on
/// the `auth` domain with the canonical source and a sanitized reason, so
/// the credentials subscriber can drive reauth.
#[tokio::test]
async fn publish_backend_session_expired_emits_sanitized_session_expired() {
    use crate::core::events::DomainEvent;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();

    // `TEST_MARKER_A` makes this event distinguishable from the sibling
    // `chat_completions_backend_401_*` test's event on the shared global
    // bus (both run in parallel against the same singleton). The `sk-`
    // token probes that `sanitize_api_error` actually scrubs secrets out
    // of the SessionExpired reason rather than just emitting the event.
    let secret = ["sk-", "SYNTHETIC", "000000000000000000000000"].concat();
    let msg = format!(
        r#"OpenHuman API error (401 Unauthorized): {{"success":false,"error":"TEST_MARKER_A Invalid token {secret}"}}"#
    );
    publish_backend_session_expired(
        "chat_completions",
        "OpenHuman",
        reqwest::StatusCode::UNAUTHORIZED,
        &msg,
    );

    let mut reason_seen: Option<String> = None;
    loop {
        match rx.try_recv() {
            Ok(DomainEvent::SessionExpired { source, reason, .. }) => {
                if source == "llm_provider.openhuman_backend" && reason.contains("TEST_MARKER_A") {
                    reason_seen = Some(reason);
                    break;
                }
            }
            Ok(_) => continue,
            Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    let reason = reason_seen.expect(
        "publish_backend_session_expired must emit SessionExpired(source=llm_provider.openhuman_backend) carrying TEST_MARKER_A",
    );
    assert!(
        reason.contains("[REDACTED]"),
        "sanitize_api_error must redact the sk- token in the reason: {reason}"
    );
    assert!(
        !reason.contains(&secret),
        "raw secret must not survive into the SessionExpired reason: {reason}"
    );
}

#[test]
fn skips_sentry_report_for_transient_upstream_statuses() {
    // Transient statuses — 429 rate-limit, 408 client timeout, and 502/503/504
    // gateway-layer failures — are retried by reliable.rs. The aggregate
    // "all providers exhausted" event still fires for genuine outages.
    // Reporting each attempt individually floods Sentry (OPENHUMAN-TAURI-2E
    // ~1393 events, 84 ~1050 events, T ~871 events).
    for transient in [
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        reqwest::StatusCode::REQUEST_TIMEOUT,
        reqwest::StatusCode::BAD_GATEWAY,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        reqwest::StatusCode::GATEWAY_TIMEOUT,
    ] {
        assert!(
            !should_report_provider_http_failure(transient),
            "transient status {transient} must not trigger per-attempt Sentry report"
        );
    }
    // Auth + permanent server faults remain reportable — those are
    // misconfiguration or genuine bugs, not transient capacity issues.
    for reportable in [
        reqwest::StatusCode::UNAUTHORIZED,
        reqwest::StatusCode::FORBIDDEN,
        reqwest::StatusCode::BAD_REQUEST,
        reqwest::StatusCode::NOT_FOUND,
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        assert!(
            should_report_provider_http_failure(reportable),
            "status {reportable} must still report to Sentry"
        );
    }
}

#[test]
fn provider_http_policy_uses_registered_error_ownership_metadata() {
    use crate::core::slots::Extensions;
    use crate::inference::registry::{
        register_backend_error_classifier_in, BackendErrorClassification, BackendErrorKind,
    };
    use std::sync::Arc;

    const PROVIDER: &str = "fixture-managed-provider";
    const OWNED: &str = "owned fixture";
    const REPORTABLE: &str = "reportable fixture";
    let extensions = Extensions::new();
    register_backend_error_classifier_in(
        &extensions,
        Arc::new(|provider, message| {
            let is_owned = provider == Some(PROVIDER) && message == OWNED;
            let is_reportable = provider == Some(PROVIDER) && message == REPORTABLE;
            BackendErrorClassification {
                provider_owned: is_owned || is_reportable,
                backend_owned: is_owned || is_reportable,
                reportable: !is_owned,
                token: None,
                kind: is_reportable.then_some(BackendErrorKind::Internal),
                malformed_bad_request: false,
            }
        }),
    );
    let context = crate::core::runtime::CoreContext::for_test_with_extensions(
        crate::core::runtime::DomainSet::full(),
        None,
        extensions,
    );

    crate::core::runtime::CoreContext::sync_scope(context, || {
        assert!(is_backend_error_code_owned(PROVIDER, OWNED));
        assert!(!is_backend_error_code_owned(PROVIDER, REPORTABLE));
        assert!(!is_backend_error_code_owned("fixture-byo-provider", OWNED));
    });
}

#[test]
fn quota_demotion_precedes_backend_owned_error_code_guard() {
    use crate::core::slots::Extensions;
    use crate::inference::registry::{
        register_backend_error_classifier_in, BackendErrorClassification,
    };
    use std::sync::Arc;
    use tinyinference_llm::failure::demotion::DemotionKind;
    const PROVIDER: &str = "fixture-owned-code-provider";
    const BODY: &str = "MONTHLY_REQUEST_COUNT quota exceeded OWNED_CODE";
    let extensions = Extensions::new();
    register_backend_error_classifier_in(
        &extensions,
        Arc::new(|provider, message| BackendErrorClassification {
            provider_owned: provider == Some(PROVIDER),
            backend_owned: provider == Some(PROVIDER) && message.contains("OWNED_CODE"),
            reportable: false,
            token: None,
            kind: None,
            malformed_bad_request: false,
        }),
    );
    let context = crate::core::runtime::CoreContext::for_test_with_extensions(
        crate::core::runtime::DomainSet::full(),
        None,
        extensions,
    );
    crate::core::runtime::CoreContext::sync_scope(context, || {
        let facts = tinyinference_llm::failure::http::classify_http_failure(
            PROVIDER,
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            BODY,
        );
        assert!(is_backend_error_code_owned(PROVIDER, BODY));
        assert_eq!(
            select_demotion(&facts, true, is_backend_error_code_owned(PROVIDER, BODY)),
            HostDemotion::Provider(DemotionKind::QuotaExhausted)
        );

        let later_provider_state = "insufficient credits OWNED_CODE";
        let later_facts = tinyinference_llm::failure::http::classify_http_failure(
            PROVIDER,
            StatusCode::PAYMENT_REQUIRED.as_u16(),
            later_provider_state,
        );
        assert_eq!(
            select_demotion(
                &later_facts,
                true,
                is_backend_error_code_owned(PROVIDER, later_provider_state)
            ),
            HostDemotion::BackendOwned
        );
    });
}

#[test]
fn managed_unknown_model_remains_user_configuration_state() {
    use tinyinference_llm::failure::demotion::{demote_decision, DemotionKind};
    let facts = tinyinference_llm::failure::http::classify_http_failure(
        "openhuman",
        StatusCode::BAD_REQUEST.as_u16(),
        "Model 'custom:MiniMax-M2.7' is not available. Use GET /openai/v1/models to list available models.",
    );
    assert_eq!(
        demote_decision(&facts, true),
        Some(DemotionKind::ConfigurationRejected)
    );
}

#[test]
fn managed_provider_config_phrase_remains_reportable_but_unknown_model_demotes() {
    use tinyinference_llm::failure::demotion::{demote_decision, DemotionKind};
    let config_phrase = tinyinference_llm::failure::http::classify_http_failure(
        "openhuman",
        StatusCode::BAD_REQUEST.as_u16(),
        "invalid temperature: only 1 is allowed for this model",
    );
    assert_eq!(demote_decision(&config_phrase, true), None);

    let third_party_phrase = tinyinference_llm::failure::http::classify_http_failure(
        "custom_openai",
        StatusCode::BAD_REQUEST.as_u16(),
        "invalid temperature: only 1 is allowed for this model",
    );
    assert_eq!(
        demote_decision(&third_party_phrase, false),
        Some(DemotionKind::ConfigurationRejected)
    );
}

#[test]
fn config_rejection_demotion_keeps_provider_status_and_polarity() {
    use tinyinference_llm::failure::demotion::{demote_decision, DemotionKind};
    let cases = [
        (
            "custom_openai",
            StatusCode::BAD_REQUEST,
            "The supported API model names are deepseek-v4-pro or deepseek-v4-flash, but you passed reasoning-v1.",
            false,
            Some(DemotionKind::ConfigurationRejected),
        ),
        (
            "custom_openai",
            StatusCode::BAD_REQUEST,
            "invalid temperature: only 1 is allowed for this model",
            false,
            Some(DemotionKind::ConfigurationRejected),
        ),
        (
            "custom_openai",
            StatusCode::NOT_FOUND,
            "The model `gpt-5.5` does not exist or you do not have access to it.",
            false,
            Some(DemotionKind::ConfigurationRejected),
        ),
        (
            "ollama",
            StatusCode::FORBIDDEN,
            "this model requires a subscription, upgrade for access",
            false,
            Some(DemotionKind::ConfigurationRejected),
        ),
        (
            "custom_openai",
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid temperature: only 1 is allowed for this model",
            false,
            None,
        ),
        (
            "custom_openai",
            StatusCode::TOO_MANY_REQUESTS,
            "invalid temperature: only 1 is allowed for this model",
            false,
            None,
        ),
        (
            "custom_openai",
            StatusCode::BAD_REQUEST,
            "Bad request: missing required field 'messages'",
            false,
            None,
        ),
        (
            "openhuman",
            StatusCode::FORBIDDEN,
            "this model requires a subscription, upgrade for access",
            true,
            None,
        ),
    ];
    for (provider, status, body, managed, expected) in cases {
        let facts = tinyinference_llm::failure::http::classify_http_failure(
            provider,
            status.as_u16(),
            body,
        );
        assert_eq!(
            demote_decision(&facts, managed),
            expected,
            "provider={provider}, status={status}, body={body}"
        );
    }
}
