use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_required_inputs,
};

#[test]
fn credential_schema_wire_contracts_are_preserved() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "auth");
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "set_credential",
    );
    let key_contracts = [
        ("auth_get_state", "get_state"),
        ("auth_get_session_token", "get_session_token"),
        (
            "auth_store_provider_credentials",
            "store_provider_credentials",
        ),
        (
            "auth_remove_provider_credentials",
            "remove_provider_credentials",
        ),
    ];
    for (key, expected_function) in key_contracts {
        let schema = schemas(key);
        expect_schema_identity(&schema, "auth", expected_function);
    }
    let set = schemas("auth_set_credential");
    expect_schema_identity(&set, "auth", "set_credential");
    expect_schema_required_inputs(&set, &["token"]);
    expect_schema_fields(&set, &[("kind", None), ("userId", None), ("user", None)]);

    let clear = schemas("auth_clear_credential");
    expect_schema_identity(&clear, "auth", "clear_credential");
    expect_schema_required_inputs(&clear, &[]);
    expect_schema_fields(&clear, &[("kind", Some(false))]);

    let list = schemas("auth_list_provider_credentials");
    expect_schema_identity(&list, "auth", "list_provider_credentials");
    expect_schema_fields(&list, &[("provider", Some(false))]);
}

// ── Param-deserialization helper ───────────────────────────────

#[test]
fn deserialize_params_parses_set_credential_request() {
    let mut m = Map::new();
    m.insert("token".into(), Value::String("abc".into()));
    let parsed: crate::security::credentials::SetCredentialRequest = deserialize_params(m).unwrap();
    assert_eq!(parsed.token, "abc");
    assert!(parsed.kind.is_none());
    assert!(parsed.user_id.is_none());
    assert!(parsed.user.is_none());
}

#[test]
fn deserialize_params_honours_userid_camel_and_snake_case() {
    for key in ["userId", "user_id"] {
        let mut m = Map::new();
        m.insert("token".into(), Value::String("abc".into()));
        m.insert(key.into(), Value::String("u1".into()));
        m.insert("kind".into(), Value::String("session".into()));
        let parsed: crate::security::credentials::SetCredentialRequest =
            deserialize_params(m).unwrap();
        assert_eq!(parsed.user_id.as_deref(), Some("u1"), "{key}");
        assert_eq!(parsed.kind.as_deref(), Some("session"));
    }
}

#[test]
fn deserialize_params_reports_missing_required_fields() {
    // `token` is required — an empty object must fail.
    let err = deserialize_params::<crate::security::credentials::SetCredentialRequest>(Map::new())
        .unwrap_err();
    assert!(err.contains("invalid params"));
}

#[test]
fn deserialize_params_parses_clear_credential_kind() {
    let parsed: AuthClearCredentialParams = deserialize_params(Map::new()).unwrap();
    assert!(parsed.kind.is_none());
    let mut m = Map::new();
    m.insert("kind".into(), Value::String("api-key".into()));
    let parsed: AuthClearCredentialParams = deserialize_params(m).unwrap();
    assert_eq!(parsed.kind.as_deref(), Some("api-key"));
}

#[test]
fn deserialize_params_parses_optional_provider_filter() {
    // Empty object is legal (provider is optional).
    let parsed: AuthListProviderCredentialsParams = deserialize_params(Map::new()).unwrap();
    assert!(parsed.provider.is_none());

    let mut m = Map::new();
    m.insert("provider".into(), Value::String("openai".into()));
    let parsed: AuthListProviderCredentialsParams = deserialize_params(m).unwrap();
    assert_eq!(parsed.provider.as_deref(), Some("openai"));
}

// ── RPC-outcome serializer ─────────────────────────────────────

#[test]
fn to_json_emits_logs_and_result_envelope() {
    let outcome = Outcome::single_log(serde_json::json!({"ok": true}), "my-log");
    let v = to_json(outcome).unwrap();
    // `into_cli_compatible_json` wraps Outcome as `{logs, result}`.
    assert!(v.get("logs").is_some(), "expected a `logs` field: {v}");
    assert!(
        v.get("result").is_some(),
        "expected a `result` envelope for the data: {v}"
    );
    assert_eq!(v["logs"][0], "my-log");
    assert_eq!(v["result"]["ok"], true);
}
