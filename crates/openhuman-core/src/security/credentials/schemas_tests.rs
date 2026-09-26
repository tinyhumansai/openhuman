use super::*;

// ── Schema catalog coverage ────────────────────────────────────

#[test]
fn catalog_counts_match() {
    let schemas = all_controller_schemas();
    let handlers = all_registered_controllers();
    assert_eq!(schemas.len(), handlers.len());
    // The account-bound `auth.oauth_*` / `auth.create_channel_link_token`
    // controllers moved to `openhuman-tinyhumans`; the core keeps the rest.
    assert!(schemas.len() >= 8, "auth namespace should expose ≥8 core fns");
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
        "auth_set_credential",
        "auth_clear_credential",
        "auth_get_state",
        "auth_get_session_token",
        "auth_store_provider_credentials",
        "auth_remove_provider_credentials",
        "auth_list_provider_credentials",
        "auth_oauth_fetch_client_key",
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
fn set_credential_schema_requires_token_and_advertises_kind_user_fields() {
    let s = schemas("auth_set_credential");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["token"]);
    for name in ["kind", "userId", "user"] {
        assert!(s.inputs.iter().any(|f| f.name == name), "missing {name}");
    }
    let clear = schemas("auth_clear_credential");
    assert!(clear.inputs.iter().all(|f| !f.required));
    assert!(clear.inputs.iter().any(|f| f.name == "kind"));
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
