use serde_json::json;

use super::{
    build_http_schema_dump, default_state, escape_html, invoke_method, is_session_expired_error,
    params_to_object, parse_json_params, type_name,
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
async fn invoke_autocomplete_status_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.autocomplete_status",
        json!({ "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
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
async fn invoke_memory_list_namespaces_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.memory_list_namespaces",
        json!({ "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("extra"));
}

#[tokio::test]
async fn invoke_memory_query_namespace_missing_namespace_fails() {
    let err = invoke_method(
        default_state(),
        "openhuman.memory_query_namespace",
        json!({ "query": "who owns atlas" }),
    )
    .await
    .expect_err("missing namespace should fail");
    assert!(err.contains("namespace"));
}

#[tokio::test]
async fn invoke_memory_recall_memories_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.memory_recall_memories",
        json!({ "namespace": "team", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("extra"));
}

#[tokio::test]
async fn invoke_migrate_openclaw_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.migrate_openclaw",
        json!({ "x": 1 }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'x'"));
}

#[tokio::test]
async fn invoke_local_ai_download_asset_missing_required_param_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.local_ai_download_asset",
        json!({}),
    )
    .await
    .expect_err("missing capability should fail");
    assert!(err.contains("missing required param 'capability'"));
}

#[test]
fn http_schema_dump_includes_openhuman_and_core_methods() {
    let dump = build_http_schema_dump();
    let methods = dump.methods;
    assert!(
        methods
            .iter()
            .any(|m| m.method == "core.version" && m.namespace == "core"),
        "schema dump should include core methods"
    );

    assert!(
        methods
            .iter()
            .any(|m| m.method == "openhuman.health_snapshot"),
        "schema dump should include migrated openhuman methods"
    );

    assert!(
        methods
            .iter()
            .any(|m| m.method == "openhuman.billing_get_current_plan"),
        "schema dump should include billing methods"
    );

    assert!(
        methods
            .iter()
            .any(|m| m.method == "openhuman.team_list_members"),
        "schema dump should include team methods"
    );
}

#[tokio::test]
async fn billing_get_current_plan_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_get_current_plan",
        json!({ "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn billing_purchase_plan_missing_plan_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_purchase_plan",
        json!({}),
    )
    .await
    .expect_err("missing plan should fail");
    assert!(err.contains("missing required param 'plan'"));
}

#[tokio::test]
async fn billing_top_up_missing_amount_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.billing_top_up", json!({}))
        .await
        .expect_err("missing amountUsd should fail");
    assert!(err.contains("missing required param 'amountUsd'"));
}

#[tokio::test]
async fn billing_top_up_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_top_up",
        json!({ "amountUsd": 10.0, "unknownField": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'unknownField'"));
}

#[tokio::test]
async fn billing_create_portal_session_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_create_portal_session",
        json!({ "x": 1 }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'x'"));
}

#[tokio::test]
async fn team_list_members_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_list_members", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_list_members_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_list_members",
        json!({ "teamId": "t1", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn team_create_invite_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_create_invite", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_remove_member_missing_required_params_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_remove_member",
        json!({ "teamId": "t1" }),
    )
    .await
    .expect_err("missing userId should fail");
    assert!(err.contains("missing required param 'userId'"));
}

#[tokio::test]
async fn team_change_member_role_missing_role_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_change_member_role",
        json!({ "teamId": "t1", "userId": "u1" }),
    )
    .await
    .expect_err("missing role should fail");
    assert!(err.contains("missing required param 'role'"));
}

#[tokio::test]
async fn billing_create_coinbase_charge_missing_plan_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_create_coinbase_charge",
        json!({}),
    )
    .await
    .expect_err("missing plan should fail");
    assert!(err.contains("missing required param 'plan'"));
}

#[tokio::test]
async fn billing_create_coinbase_charge_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_create_coinbase_charge",
        json!({ "plan": "pro", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn team_list_invites_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_list_invites", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_list_invites_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_list_invites",
        json!({ "teamId": "t1", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn team_revoke_invite_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_revoke_invite", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_revoke_invite_missing_invite_id_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_revoke_invite",
        json!({ "teamId": "t1" }),
    )
    .await
    .expect_err("missing inviteId should fail");
    assert!(err.contains("missing required param 'inviteId'"));
}

#[tokio::test]
async fn schema_dump_includes_new_billing_and_team_methods() {
    let dump = build_http_schema_dump();
    let methods: Vec<&str> = dump.methods.iter().map(|m| m.method.as_str()).collect();
    for expected in &[
        "openhuman.billing_get_current_plan",
        "openhuman.billing_purchase_plan",
        "openhuman.billing_create_portal_session",
        "openhuman.billing_top_up",
        "openhuman.billing_create_coinbase_charge",
        "openhuman.team_list_members",
        "openhuman.team_create_invite",
        "openhuman.team_list_invites",
        "openhuman.team_revoke_invite",
        "openhuman.team_remove_member",
        "openhuman.team_change_member_role",
    ] {
        assert!(
            methods.contains(expected),
            "schema dump missing expected method: {expected}"
        );
    }
}

// --- helper coverage -----------------------------------------------------

#[test]
fn params_to_object_accepts_object() {
    let map = params_to_object(json!({"a": 1, "b": "x"})).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("a"), Some(&json!(1)));
}

#[test]
fn params_to_object_accepts_null_as_empty_map() {
    let map = params_to_object(json!(null)).unwrap();
    assert!(map.is_empty());
}

#[test]
fn params_to_object_rejects_array() {
    let err = params_to_object(json!([1, 2, 3])).unwrap_err();
    assert!(err.contains("invalid params"));
    assert!(err.contains("array"));
}

#[test]
fn params_to_object_rejects_scalars() {
    assert!(params_to_object(json!(42)).unwrap_err().contains("number"));
    assert!(params_to_object(json!("hi"))
        .unwrap_err()
        .contains("string"));
    assert!(params_to_object(json!(true)).unwrap_err().contains("bool"));
}

#[test]
fn type_name_labels_every_json_variant() {
    assert_eq!(type_name(&json!(null)), "null");
    assert_eq!(type_name(&json!(true)), "bool");
    assert_eq!(type_name(&json!(3)), "number");
    assert_eq!(type_name(&json!("s")), "string");
    assert_eq!(type_name(&json!([])), "array");
    assert_eq!(type_name(&json!({})), "object");
}

#[test]
fn parse_json_params_roundtrips_object() {
    let v = parse_json_params(r#"{"k":1}"#).unwrap();
    assert_eq!(v, json!({"k": 1}));
}

#[test]
fn parse_json_params_reports_error_message() {
    let err = parse_json_params("{not json").unwrap_err();
    assert!(err.contains("invalid JSON params"));
}

#[test]
fn is_session_expired_error_matches_401_unauthorized() {
    assert!(is_session_expired_error(
        "backend returned 401 Unauthorized"
    ));
    assert!(is_session_expired_error("401 UNAUTHORIZED"));
    assert!(is_session_expired_error("got 401 and unauthorized body"));
}

#[test]
fn is_session_expired_error_requires_both_401_and_unauthorized() {
    // 401 alone is not sufficient — could be HTTP/3.01 nonsense or
    // unrelated text. We require the string "unauthorized" too.
    assert!(!is_session_expired_error("server returned 401"));
    assert!(!is_session_expired_error("unauthorized without code"));
}

#[test]
fn is_session_expired_error_matches_invalid_token_case_insensitive() {
    assert!(is_session_expired_error("Invalid Token"));
    assert!(is_session_expired_error("got an invalid token here"));
}

#[test]
fn is_session_expired_error_matches_session_expired_sentinel() {
    // The SESSION_EXPIRED sentinel is case-sensitive by design.
    assert!(is_session_expired_error("SESSION_EXPIRED: please re-auth"));
    assert!(!is_session_expired_error("session_expired lowercase"));
}

#[test]
fn is_session_expired_error_does_not_match_unrelated_errors() {
    assert!(!is_session_expired_error("network timeout"));
    assert!(!is_session_expired_error("500 internal server error"));
    assert!(!is_session_expired_error(""));
}

#[test]
fn escape_html_escapes_all_special_chars() {
    let raw = r#"<script>alert("x&y'z")</script>"#;
    let escaped = escape_html(raw);
    assert!(!escaped.contains('<'));
    assert!(!escaped.contains('>'));
    assert!(!escaped.contains('"'));
    assert!(!escaped.contains('\''));
    assert!(escaped.contains("&lt;"));
    assert!(escaped.contains("&gt;"));
    assert!(escaped.contains("&quot;"));
    assert!(escaped.contains("&#x27;"));
    // `&` must be escaped first so later substitutions don't double-encode.
    assert!(escaped.contains("&amp;y"));
}

#[test]
fn escape_html_is_noop_for_safe_text() {
    assert_eq!(escape_html("safe text 123"), "safe text 123");
    assert_eq!(escape_html(""), "");
}

// --- invoke_method parameter-shape errors ---------------------------------

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
