use serde_json::json;

use super::super::session_expiry::is_session_expired_error;
use super::{
    default_state, invoke_method, should_publish_session_expired, translate_local_session_error,
};

#[tokio::test]
async fn invoke_health_snapshot_via_registry() {
    let result = invoke_method(default_state(), "openhuman.health_snapshot", json!({}))
        .await
        .expect("health snapshot should succeed");
    assert!(result.get("result").is_some());
}

#[tokio::test]
async fn invoke_encrypt_secret_missing_required_param_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.encrypt_secret", json!({}))
        .await
        .expect_err("missing plaintext should fail");
    assert!(err.contains("missing required param 'plaintext'"));
}

#[tokio::test]
async fn invoke_doctor_models_rejects_unknown_param() {
    let err = invoke_method(
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
        let err = CoreContext::scope(
            ctx,
            invoke_method(default_state(), &gated_method, params.clone()),
        )
        .await
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
    let err = invoke_method(default_state(), "openhuman.auth_store_session", json!({}))
        .await
        .expect_err("missing token should fail");
    assert!(err.contains("missing required param 'token'"));
}

#[tokio::test]
async fn invoke_service_status_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.service_status",
        json!({ "x": 1 }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'x'"));
}

#[tokio::test]
async fn invoke_memory_init_accepts_empty_params() {
    // jwt_token is optional (accepted for backward compat but ignored).
    // The call may still fail for workspace reasons in test, but must NOT
    // fail with a missing-param error for jwt_token.
    let result = invoke_method(default_state(), "openhuman.memory_init", json!({})).await;
    if let Err(ref e) = result {
        assert!(
            !e.contains("missing required param") || !e.contains("jwt_token"),
            "jwt_token should be optional, got: {e}"
        );
    }
}

#[tokio::test]
async fn invoke_memory_learn_rejects_unknown_param() {
    let err = invoke_method(
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
    let local = crate::security::credentials::session_support::is_local_session_token(
        "header.payload.local",
    );
    let jwt = crate::security::credentials::session_support::is_local_session_token(
        "header.payload.signature",
    );
    for error in [
        "composio unavailable: no backend session token. Sign in first (auth_store_session).",
        "SESSION_EXPIRED: backend rejected session token on GET /teams/me/usage",
    ] {
        assert!(!should_publish_session_expired(error, local), "{error}");
        assert!(should_publish_session_expired(error, jwt), "{error}");
        let translated = translate_local_session_error(error, local).expect("local fallback");
        assert!(
            translated.starts_with(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
            "{translated}"
        );
        assert!(!is_session_expired_error(&translated));
        assert!(translate_local_session_error(error, jwt).is_none());
    }
}

#[tokio::test]
async fn invoke_method_rejects_array_params_for_registered_method() {
    // Registered controllers expect named-argument style (JSON object).
    // Passing an array must fail with a clear "invalid params" error
    // instead of silently calling the handler with no args.
    let err = invoke_method(
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
    let err = invoke_method(default_state(), "openhuman.health_snapshot", json!("oops"))
        .await
        .expect_err("string params should be rejected");
    assert!(err.contains("invalid params"));
    assert!(err.contains("string"));
}

#[tokio::test]
async fn invoke_method_accepts_null_params_for_registered_method() {
    // JSON-RPC 2.0 allows omitting params; null must be treated like {}.
    let result = invoke_method(default_state(), "openhuman.health_snapshot", json!(null)).await;
    // Call should succeed or fail for domain reasons — but must NOT
    // fail with the "invalid params" shape error.
    if let Err(e) = result {
        assert!(
            !e.contains("invalid params"),
            "null should be accepted as empty object, got: {e}"
        );
    }
}

#[tokio::test]
async fn invoke_method_unknown_method_returns_unknown_error() {
    let err = invoke_method(default_state(), "openhuman.totally_made_up_xyz", json!({}))
        .await
        .expect_err("unknown methods must error");
    assert!(err.contains("unknown method"));
}

#[tokio::test]
async fn invoke_method_core_ping_via_tier1() {
    // core.* methods aren't in the registry; they route through tier 1.
    let result = invoke_method(default_state(), "core.ping", json!({}))
        .await
        .expect("core.ping should succeed via tier 1");
    assert_eq!(result, json!({ "ok": true }));
}

#[tokio::test]
async fn invoke_method_core_version_via_tier1_reflects_state() {
    let state = super::AppState {
        core_version: "0.0.1-abc".into(),
    };
    let result = invoke_method(state, "core.version", json!({}))
        .await
        .expect("core.version should succeed");
    assert_eq!(result, json!({ "version": "0.0.1-abc" }));
}
