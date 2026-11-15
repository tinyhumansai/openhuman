use crate::core::runtime::DispatchCtx;
use serde_json::json;

use super::{
    default_state, invoke_method, is_unconfirmed_unauthorized_error,
    request_session_failure_is_actionable, session_failure_outcome, translate_local_session_error,
    SessionFailureOutcome,
};
use crate::core::observability::is_session_expired_error;

#[tokio::test]
async fn invoke_health_snapshot_via_registry() {
    let result = invoke_method(
        DispatchCtx::ambient(),
        default_state(),
        "openhuman.health_snapshot",
        json!({}),
    )
    .await
    .expect("health snapshot should succeed");
    assert!(result.get("result").is_some());
}

#[tokio::test]
async fn invoke_encrypt_secret_missing_required_param_fails_validation() {
    let err = invoke_method(
        DispatchCtx::ambient(),
        default_state(),
        "openhuman.encrypt_secret",
        json!({}),
    )
    .await
    .expect_err("missing plaintext should fail");
    assert!(err.contains("missing required param 'plaintext'"));
}

#[tokio::test]
async fn invoke_doctor_models_rejects_unknown_param() {
    let err = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.doctor_models",
        json!({ "invalid": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'invalid'"));
}

// Uses a `flows.*` method as its gated-family vehicle: without the `flows`
// feature there is no flows controller in the registry and the `.expect()`
// below would panic. The transport-layer gating it proves is orthogonal to the
// compile-time gate (#4797).
#[cfg(feature = "flows")]
#[tokio::test]
async fn gated_method_is_unknown_at_transport_even_with_malformed_params() {
    // #4808 review (CodeRabbit): prove the schema-gate fix at the JSON-RPC
    // TRANSPORT layer (`invoke_method`), not only via direct dispatch. Under a
    // harness() ambient context a gated method must return an unknown-method
    // error for BOTH well-formed and malformed params — never the controller's
    // param-validation error, which would leak that the hidden method exists.
    use crate::core::runtime::context::CoreContext;
    use crate::core::runtime::DomainSet;

    let gated_method = crate::core::all::all_registered_controllers()
        .into_iter()
        .find(|c| c.schema.namespace == "flows")
        .map(|c| c.rpc_method_name())
        .expect("a flows.* method exists in the full registry");

    for params in [json!({}), json!({ "obviously_not_a_real_param_xyz": true })] {
        let ctx = CoreContext::for_test(DomainSet::harness(), None);
        let err = {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(ctx);
            invoke_method(
                scope_ctx.clone(),
                default_state(),
                &gated_method,
                params.clone(),
            )
            .await
        }
        .expect_err("gated flows method must error under harness()");
        assert!(
            err.contains("unknown method"),
            "gated `{gated_method}` with params {params} must be unknown-method at transport, got: {err}"
        );
        assert!(
            !err.contains("param"),
            "gated `{gated_method}` must NOT leak a param-validation error (surface leak), got: {err}"
        );
    }
}

#[tokio::test]
async fn invoke_config_get_runtime_flags_via_registry() {
    let result = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.config_get_runtime_flags",
        json!({}),
    )
    .await
    .expect("runtime flags should succeed");
    assert!(result.get("result").is_some());
}

#[tokio::test]
async fn invoke_auth_store_session_missing_token_fails_validation() {
    let err = invoke_method(
        DispatchCtx::ambient(),
        default_state(),
        "openhuman.auth_store_session",
        json!({}),
    )
    .await
    .expect_err("missing token should fail");
    assert!(err.contains("missing required param 'token'"));
}

#[tokio::test]
async fn invoke_service_status_rejects_unknown_param() {
    let err = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.service_status",
        json!({ "x": 1 }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'x'"));
}

#[tokio::test]
async fn invoke_memory_learn_rejects_unknown_param() {
    let err = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.memory_learn",
        json!({ "text": "prefers tea", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("extra"), "{err}");
}

#[tokio::test]
async fn invoke_memory_recall_missing_question_fails() {
    let err = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.memory_recall",
        json!({ "limit": 3 }),
    )
    .await
    .expect_err("missing question should fail");
    assert!(err.contains("question"), "{err}");
}

#[tokio::test]
async fn invoke_retired_v1_memory_method_is_unknown() {
    let err = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.memory_recall_memories",
        json!({ "namespace": "team" }),
    )
    .await
    .expect_err("v1 method is gone");
    assert!(err.contains("memory_recall_memories"), "{err}");
}

#[test]
fn local_offline_credential_does_not_publish_backend_session_expiry() {
    let local = crate::credentials::session_state::is_local_session_token("header.payload.local");
    let jwt = crate::credentials::session_state::is_local_session_token("header.payload.signature");
    for error in [
        "composio unavailable: no backend session token. Sign in first (auth_store_session).",
        "SESSION_EXPIRED: backend rejected session token on GET /teams/me/usage",
    ] {
        assert!(
            !request_session_failure_is_actionable(
                error,
                local,
                None,
                Some("header.payload.local")
            ),
            "{error}"
        );
        assert!(
            !request_session_failure_is_actionable(
                error,
                jwt,
                None,
                Some("header.payload.signature")
            ),
            "generic invoke failures lack request-time provenance: {error}"
        );
        let translated = translate_local_session_error(error, local).expect("local fallback");
        assert!(
            translated.starts_with(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
            "{translated}"
        );
        assert!(!is_session_expired_error(&translated));
        assert!(translate_local_session_error(error, jwt).is_none());
    }
}

#[test]
fn rpc_expiry_requires_matching_request_time_credential_provenance() {
    let request_fingerprint =
        crate::credentials::backend_credential::credential_fingerprint("request-token");
    let error = "SESSION_EXPIRED: backend rejected session token";
    assert!(!request_session_failure_is_actionable(
        error,
        false,
        None,
        Some("replacement-token"),
    ));
    assert!(!request_session_failure_is_actionable(
        error,
        false,
        Some(&request_fingerprint),
        Some("replacement-token"),
    ));
    assert!(request_session_failure_is_actionable(
        error,
        false,
        Some(&request_fingerprint),
        Some("request-token"),
    ));
}

/// A signed-out core (no stored session, no API key) answers a hosted call
/// with a local pre-flight refusal. No credential was sent, so there is no
/// provenance to verify and nothing to sign out: the original error, with its
/// sign-in hint, must reach the caller untouched and no expiry is published.
#[test]
fn signed_out_preflight_refusal_passes_through_untouched() {
    for error in [
        "no backend session token; run auth_store_session first",
        "composio unavailable: no backend session token. Sign in or set a TinyHumans API key.",
    ] {
        assert!(is_session_expired_error(error), "{error}");
        assert_eq!(
            session_failure_outcome(error, false, false, None, None),
            SessionFailureOutcome::PassThrough,
            "{error}"
        );
    }
}

/// A credential that existed keeps the fail-closed rewrites: generic
/// dispatch cannot prove which token received a downstream 401.
#[test]
fn stored_credentials_keep_session_failure_rewrites() {
    let error = "SESSION_EXPIRED: backend rejected session token";
    let reason_of = |outcome| match outcome {
        SessionFailureOutcome::Replace { reason, message } => {
            assert!(!is_session_expired_error(&message), "{message}");
            reason
        }
        other => panic!("expected a replaced error, got {other:?}"),
    };
    assert_eq!(
        reason_of(session_failure_outcome(
            error,
            false,
            false,
            None,
            Some("header.payload.signature")
        )),
        "credential_provenance_unavailable"
    );
    assert_eq!(
        reason_of(session_failure_outcome(error, false, true, None, None)),
        "api_key_credential"
    );
    assert_eq!(
        reason_of(session_failure_outcome(
            error,
            true,
            false,
            None,
            Some("header.payload.local")
        )),
        "local_offline_credential"
    );
    let fingerprint =
        crate::credentials::backend_credential::credential_fingerprint("request-token");
    assert_eq!(
        session_failure_outcome(
            error,
            false,
            false,
            Some(&fingerprint),
            Some("request-token")
        ),
        SessionFailureOutcome::PublishExpiry
    );
}

/// A memory engine 401 as the memory controller encodes it: a structured
/// `UNAUTHORIZED` code the UI branches on, under a message the session-expiry
/// classifier matches.
fn structured_memory_unauthorized() -> String {
    crate::core::StructuredRpcError {
        message: "unauthorized: [UNAUTHORIZED] memory API memory/learn on api.tinyhumans.ai: \
                  the session expired or the API key was rejected (HTTP 401 Unauthorized); \
                  re-authenticate"
            .to_string(),
        data: Some(json!({ "kind": "UNAUTHORIZED", "code": "UNAUTHORIZED" })),
        expected_user_state: true,
    }
    .encode()
}

fn replaced_message(outcome: SessionFailureOutcome) -> String {
    match outcome {
        SessionFailureOutcome::Replace { message, .. } => message,
        other => panic!("expected a replaced error, got {other:?}"),
    }
}

/// The rewrite drops the auth-expired message but keeps a structured code the
/// renderer does not read as sign-out, so `memoryErrorCode` still sees
/// `UNAUTHORIZED` instead of `data: null`.
#[test]
fn replaced_session_failure_keeps_non_signout_structured_code() {
    let original = structured_memory_unauthorized();
    assert!(is_session_expired_error(&original));
    let cases = [
        (
            session_failure_outcome(&original, false, false, None, Some("header.payload.sig")),
            "Backend request failed; credential provenance could not be verified",
        ),
        (
            session_failure_outcome(&original, false, true, None, None),
            "Backend request failed while using an API-key credential",
        ),
        (
            session_failure_outcome(&original, true, false, None, Some("header.payload.local")),
            crate::core::observability::BACKEND_UNAVAILABLE_PREFIX,
        ),
    ];
    for (outcome, expected_message) in cases {
        let message = replaced_message(outcome);
        assert!(!is_session_expired_error(&message), "{message}");
        let envelope = crate::core::StructuredRpcError::decode(&message)
            .unwrap_or_else(|| panic!("structured envelope dropped: {message}"));
        assert!(
            envelope.message.starts_with(expected_message),
            "{}",
            envelope.message
        );
        assert_eq!(
            envelope.data,
            Some(json!({ "kind": "UNAUTHORIZED", "code": "UNAUTHORIZED" }))
        );
        assert!(envelope.expected_user_state);
    }
}

/// A structured error whose data itself carries a sign-out marker must not
/// reach the renderer through the rewrite, in any casing or field.
#[test]
fn replaced_session_failure_never_passes_through_a_signout_marker() {
    for data in [
        json!({ "kind": "SESSION_EXPIRED", "code": "SESSION_EXPIRED" }),
        json!({ "code": "session_expired" }),
        json!({ "code": "ENGINE", "detail": "Session expired; sign in again" }),
        json!({ "code": "ENGINE", "detail": "no backend session token" }),
        json!({ "code": "ENGINE", "detail": "session JWT required" }),
    ] {
        let original = crate::core::StructuredRpcError {
            message: "SESSION_EXPIRED: backend rejected session token".to_string(),
            data: Some(data.clone()),
            expected_user_state: false,
        }
        .encode();
        let message = replaced_message(session_failure_outcome(
            &original,
            false,
            false,
            None,
            Some("header.payload.sig"),
        ));
        assert!(!is_session_expired_error(&message), "{message}");
        let lower = message.to_ascii_lowercase();
        assert!(
            !lower.contains("session_expired") && !lower.contains("session expired"),
            "{message}"
        );
        assert!(
            crate::core::StructuredRpcError::decode(&message).is_none_or(|e| e.data.is_none()),
            "{data} leaked: {message}"
        );
    }
}

/// #8081: a Memory tab on a plan without memory got `403 MEMORY_PLAN_REQUIRED`
/// from the hosted engine, whose message said "the session expired", and the
/// user was signed out. Neither that nor a plain memory 403 may publish
/// `SessionExpired` through generic invoke dispatch because no request-time
/// credential fingerprint is available here.
#[test]
fn memory_plan_refusal_and_403_do_not_publish_session_expiry() {
    for signed_in in [false, true] {
        for error in [
            // As pinned tinymemory words it today (wire capture, 2026-10-19).
            "unauthorized: [MEMORY_PLAN_REQUIRED] memory API memory/scopes on api.tinyhumans.ai: \
             the session expired or the API key was rejected (HTTP 403 Forbidden); \
             re-authenticate — memory is not included in this account's plan; nothing was processed",
            // As tinymemory words it once its fix is pinned.
            "engine error: [MEMORY_PLAN_REQUIRED] memory API memory/scopes on api.tinyhumans.ai: \
             memory is not included in this account's plan (HTTP 403 Forbidden)",
            "unauthorized: [FORBIDDEN] memory API memory/scopes on api.tinyhumans.ai: the session \
             expired or the API key was rejected (HTTP 403 Forbidden); re-authenticate",
        ] {
            assert!(!is_session_expired_error(error), "{error}");
            assert!(!request_session_failure_is_actionable(
                error,
                !signed_in,
                None,
                Some("header.payload.signature"),
            ));
        }
    }
    let expired = "unauthorized: [UNAUTHORIZED] memory API memory/scopes on api.tinyhumans.ai: \
                   the session expired or the API key was rejected (HTTP 401 Unauthorized); \
                   re-authenticate";
    assert!(is_session_expired_error(expired));
    assert!(!request_session_failure_is_actionable(
        expired,
        false,
        None,
        Some("header.payload.signature"),
    ));
}

#[tokio::test]
async fn invoke_method_rejects_array_params_for_registered_method() {
    // Registered controllers expect named-argument style (JSON object).
    // Passing an array must fail with a clear "invalid params" error
    // instead of silently calling the handler with no args.
    let err = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.health_snapshot",
        json!([1, 2, 3]),
    )
    .await
    .expect_err("array params should be rejected");
    assert!(err.contains("invalid params"));
    assert!(err.contains("array"));
}

#[tokio::test]
async fn invoke_method_rejects_string_params_for_registered_method() {
    let err = invoke_method(
        DispatchCtx::ambient(),
        default_state(),
        "openhuman.health_snapshot",
        json!("oops"),
    )
    .await
    .expect_err("string params should be rejected");
    assert!(err.contains("invalid params"));
    assert!(err.contains("string"));
}

#[tokio::test]
async fn invoke_method_accepts_null_params_for_registered_method() {
    // JSON-RPC 2.0 allows omitting params; null must behave exactly like {}
    // (see invoke_health_snapshot_via_registry), not merely avoid one error.
    let result = invoke_method(
        DispatchCtx::ambient(),
        default_state(),
        "openhuman.health_snapshot",
        json!(null),
    )
    .await
    .expect("null params must behave like {}");
    assert!(result.get("result").is_some());
}

#[tokio::test]
async fn invoke_method_unknown_method_returns_unknown_error() {
    let err = invoke_method(
        DispatchCtx::ambient(),
        default_state(),
        "openhuman.totally_made_up_xyz",
        json!({}),
    )
    .await
    .expect_err("unknown methods must error");
    assert!(err.contains("unknown method"));
}

#[tokio::test]
async fn invoke_retired_dictation_settings_method_returns_unknown_error() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    let ctx = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let err = invoke_method(
        DispatchCtx::new(ctx),
        default_state(),
        "openhuman.config_get_dictation_settings",
        json!({}),
    )
    .await
    .expect_err("retired dictation settings method must be unknown");
    assert!(
        err.starts_with(crate::core::dispatch::UNKNOWN_METHOD_PREFIX),
        "got: {err}"
    );
}

#[tokio::test]
async fn invoke_method_core_ping_via_tier1() {
    // core.* methods aren't in the registry; they route through tier 1.
    let result = invoke_method(
        DispatchCtx::ambient(),
        default_state(),
        "core.ping",
        json!({}),
    )
    .await
    .expect("core.ping should succeed via tier 1");
    assert_eq!(result, json!({ "ok": true }));
}

#[tokio::test]
async fn invoke_method_core_version_via_tier1_reflects_state() {
    let state = super::AppState {
        core_version: "0.0.1-abc".into(),
    };
    let result = invoke_method(DispatchCtx::ambient(), state, "core.version", json!({}))
        .await
        .expect("core.version should succeed");
    assert_eq!(result, json!({ "version": "0.0.1-abc" }));
}

#[test]
fn unconfirmed_unauthorized_error_matches_generic_401_for_diagnostics_only() {
    // Generic 401+unauthorized text feeds the diagnostic-only branch — never
    // SessionExpired publication.
    for msg in [
        "backend returned 401 Unauthorized",
        "401 UNAUTHORIZED",
        "got 401 and unauthorized body",
        "Invalid Token",
        "got an invalid token here",
    ] {
        assert!(is_unconfirmed_unauthorized_error(msg), "{msg}");
        assert!(!is_session_expired_error(msg), "{msg}");
    }
    // Issue #2537: a backend 500 wrapping a Composio upstream 401 is logged
    // diagnostically, never treated as an expired session.
    let wrapped = r#"[composio] list_connections failed: Backend returned 500 Internal Server Error for GET https://backend.example.test/composio/connections: 401 {"error":{"message":"Invalid API key: ak_o1Og5*****","code":10401,"slug":"HTTP_Unauthorized","status":401}}"#;
    assert!(is_unconfirmed_unauthorized_error(wrapped));
    assert!(!is_session_expired_error(wrapped));
    assert!(!is_unconfirmed_unauthorized_error("network timeout"));
}
