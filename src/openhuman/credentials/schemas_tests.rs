use super::*;

// ── Schema catalog coverage ────────────────────────────────────

#[test]
fn catalog_counts_match() {
    let schemas = all_controller_schemas();
    let handlers = all_registered_controllers();
    assert_eq!(schemas.len(), handlers.len());
    assert!(schemas.len() >= 13, "auth namespace should expose ≥13 fns");
}

#[test]
fn all_schemas_use_auth_namespace_and_have_descriptions() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "auth", "function {}", s.function);
        assert!(!s.description.is_empty(), "function {}", s.function);
        assert!(
            !s.outputs.is_empty(),
            "function {} has no outputs",
            s.function
        );
    }
}

#[test]
fn unknown_function_returns_unknown_fallback() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "auth");
}

#[test]
fn every_registered_function_has_nonempty_schema_metadata() {
    for handler in all_registered_controllers() {
        assert!(
            !handler.schema.function.is_empty(),
            "registered controller is missing its function name"
        );
        assert_eq!(handler.schema.namespace, "auth");
    }
}

#[test]
fn every_known_schema_key_returns_a_non_unknown_schema() {
    // Exercises the full match arm in `schemas()`, pushing line
    // coverage for every branch without needing the async handler
    // to fire off HTTP.
    let keys = [
        "auth_store_session",
        "auth_clear_session",
        "auth_get_state",
        "auth_get_session_token",
        "auth_get_me",
        "auth_consume_login_token",
        "auth_create_channel_link_token",
        "auth_store_provider_credentials",
        "auth_remove_provider_credentials",
        "auth_list_provider_credentials",
        "auth_oauth_connect",
        "auth_oauth_list_integrations",
        "auth_oauth_fetch_integration_tokens",
        "auth_oauth_fetch_client_key",
        "auth_oauth_revoke_integration",
    ];
    for k in keys {
        let s = schemas(k);
        assert_eq!(s.namespace, "auth", "key `{k}` has wrong namespace");
        assert_ne!(
            s.function, "unknown",
            "key `{k}` fell through to the unknown fallback"
        );
        assert!(!s.description.is_empty(), "key `{k}` has empty description");
    }
}

#[test]
fn list_provider_credentials_schema_has_optional_provider_filter() {
    let s = schemas("auth_list_provider_credentials");
    let provider = s.inputs.iter().find(|f| f.name == "provider");
    assert!(provider.is_some(), "must expose `provider` input");
    assert!(!provider.unwrap().required);
}

#[test]
fn oauth_connect_schema_requires_provider() {
    let s = schemas("auth_oauth_connect");
    let provider = s.inputs.iter().find(|f| f.name == "provider").unwrap();
    assert!(provider.required);
}

#[test]
fn store_session_schema_requires_token_and_accepts_user_fields() {
    let s = schemas("auth_store_session");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"token"));
    // Schema uses snake_case field names (`user_id`). The RPC layer
    // tolerates `userId` via a serde alias, but the catalog surface
    // advertises the canonical snake_case form.
    assert!(s.inputs.iter().any(|f| f.name == "user_id"));
    assert!(s.inputs.iter().any(|f| f.name == "user"));
}

// ── Field-builder helpers ──────────────────────────────────────

#[test]
fn required_string_produces_required_string_field() {
    let f = required_string("provider", "comment");
    assert_eq!(f.name, "provider");
    assert!(matches!(f.ty, TypeSchema::String));
    assert!(f.required);
}

#[test]
fn optional_string_produces_option_string() {
    let f = optional_string("profile", "c");
    assert!(!f.required);
    match &f.ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::String)),
        _ => panic!("expected Option<String>"),
    }
}

#[test]
fn optional_bool_produces_option_bool() {
    let f = optional_bool("set_active", "c");
    assert!(!f.required);
    match &f.ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::Bool)),
        _ => panic!("expected Option<Bool>"),
    }
}

#[test]
fn optional_json_produces_option_json() {
    let f = optional_json("fields", "c");
    assert!(!f.required);
    match &f.ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::Json)),
        _ => panic!("expected Option<Json>"),
    }
}

#[test]
fn json_output_produces_required_json_output_field() {
    let f = json_output("result", "c");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::Json));
}

// ── Param-deserialization helper ───────────────────────────────

#[test]
fn deserialize_params_parses_valid_object_into_struct() {
    let mut m = Map::new();
    m.insert("token".into(), Value::String("abc".into()));
    let parsed: AuthStoreSessionParams = deserialize_params(m).unwrap();
    assert_eq!(parsed.token, "abc");
    assert!(parsed.user_id.is_none());
    assert!(parsed.user.is_none());
}

#[test]
fn deserialize_params_honours_userid_alias() {
    let mut m = Map::new();
    m.insert("token".into(), Value::String("abc".into()));
    m.insert("userId".into(), Value::String("u1".into()));
    let parsed: AuthStoreSessionParams = deserialize_params(m).unwrap();
    assert_eq!(parsed.user_id.as_deref(), Some("u1"));
}

#[test]
fn deserialize_params_reports_missing_required_fields() {
    // `token` is required — an empty object must fail.
    let err = deserialize_params::<AuthStoreSessionParams>(Map::new()).unwrap_err();
    assert!(err.contains("invalid params"));
}

#[test]
fn deserialize_params_parses_consume_login_token_camel_case() {
    let mut m = Map::new();
    m.insert("loginToken".into(), Value::String("tok".into()));
    let parsed: AuthConsumeLoginTokenParams = deserialize_params(m).unwrap();
    assert_eq!(parsed.login_token, "tok");
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
    let outcome = RpcOutcome::single_log(serde_json::json!({"ok": true}), "my-log");
    let v = to_json(outcome).unwrap();
    // `into_cli_compatible_json` wraps RpcOutcome as `{logs, result}`.
    assert!(v.get("logs").is_some(), "expected a `logs` field: {v}");
    assert!(
        v.get("result").is_some(),
        "expected a `result` envelope for the data: {v}"
    );
    assert_eq!(v["logs"][0], "my-log");
    assert_eq!(v["result"]["ok"], true);
}
