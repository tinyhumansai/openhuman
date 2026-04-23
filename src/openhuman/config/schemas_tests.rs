use super::*;

#[test]
fn catalog_counts_match_and_nonempty() {
    let s = all_controller_schemas();
    let h = all_registered_controllers();
    assert_eq!(s.len(), h.len());
    assert!(s.len() >= 20, "config namespace should expose ≥20 fns");
}

#[test]
fn all_schemas_use_config_namespace_and_have_descriptions() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "config", "function {}", s.function);
        assert!(!s.description.is_empty(), "function {} desc", s.function);
        assert!(!s.outputs.is_empty(), "function {} outputs", s.function);
    }
}

#[test]
fn unknown_function_returns_unknown_schema() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "config");
}

#[test]
fn every_registered_key_resolves_to_non_unknown_schema() {
    let keys = [
        "get_config",
        "update_model_settings",
        "update_memory_settings",
        "update_screen_intelligence_settings",
        "update_runtime_settings",
        "update_browser_settings",
        "resolve_api_url",
        "get_runtime_flags",
        "set_browser_allow_all",
        "workspace_onboarding_flag_exists",
        "workspace_onboarding_flag_set",
        "update_analytics_settings",
        "get_analytics_settings",
        "agent_server_status",
        "reset_local_data",
        "get_onboarding_completed",
        "set_onboarding_completed",
        "get_dictation_settings",
        "update_dictation_settings",
        "get_voice_server_settings",
        "update_voice_server_settings",
    ];
    for k in keys {
        let s = schemas(k);
        assert_ne!(s.function, "unknown", "`{k}` fell through to unknown");
        assert_eq!(s.namespace, "config");
    }
}

#[test]
fn registered_controllers_all_use_config_namespace() {
    for h in all_registered_controllers() {
        assert_eq!(h.schema.namespace, "config");
        assert!(!h.schema.function.is_empty());
    }
}

#[test]
fn json_output_helper_builds_required_json_field() {
    let f = json_output("result", "desc");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::Json));
}

#[test]
fn to_json_wraps_rpc_outcome() {
    let v =
        to_json(RpcOutcome::single_log(serde_json::json!({"ok": true}), "l")).expect("serialize");
    assert!(v.get("logs").is_some() || v.get("result").is_some());
}

// ── Field builder helpers ────────────────────────────────────

#[test]
fn required_string_builds_required_string_field() {
    let f = required_string("api_key", "Auth key");
    assert_eq!(f.name, "api_key");
    assert_eq!(f.comment, "Auth key");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::String));
}

#[test]
fn optional_string_builds_option_string_field() {
    let f = optional_string("model", "model name");
    assert!(!f.required);
    match &f.ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::String)),
        other => panic!("expected Option<String>, got {other:?}"),
    }
}

#[test]
fn optional_bool_builds_option_bool_field() {
    let f = optional_bool("enabled", "Whether enabled");
    assert!(!f.required);
    match &f.ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::Bool)),
        other => panic!("expected Option<Bool>, got {other:?}"),
    }
}

// ── deserialize_params helper ────────────────────────────────

#[test]
fn deserialize_params_parses_model_settings_update() {
    let mut m = Map::new();
    m.insert(
        "default_temperature".into(),
        Value::Number(serde_json::Number::from_f64(0.7).unwrap()),
    );
    let out: ModelSettingsUpdate = deserialize_params(m).unwrap();
    assert_eq!(out.default_temperature, Some(0.7));
    assert!(out.api_url.is_none());
    assert!(out.default_model.is_none());
}

#[test]
fn deserialize_params_parses_memory_settings_update() {
    let mut m = Map::new();
    m.insert("backend".into(), Value::String("sqlite".into()));
    m.insert("auto_save".into(), Value::Bool(true));
    m.insert(
        "embedding_dimensions".into(),
        Value::Number(serde_json::Number::from(1536)),
    );
    let out: MemorySettingsUpdate = deserialize_params(m).unwrap();
    assert_eq!(out.backend.as_deref(), Some("sqlite"));
    assert_eq!(out.auto_save, Some(true));
    assert_eq!(out.embedding_dimensions, Some(1536));
}

#[test]
fn deserialize_params_parses_workspace_onboarding_flag_params() {
    let out: WorkspaceOnboardingFlagParams = deserialize_params(Map::new()).unwrap();
    assert!(out.flag_name.is_none());

    let mut m = Map::new();
    m.insert("flag_name".into(), Value::String(".custom_marker".into()));
    let out: WorkspaceOnboardingFlagParams = deserialize_params(m).unwrap();
    assert_eq!(out.flag_name.as_deref(), Some(".custom_marker"));
}

#[test]
fn deserialize_params_parses_workspace_onboarding_flag_set_params() {
    let mut m = Map::new();
    m.insert("value".into(), Value::Bool(true));
    let out: WorkspaceOnboardingFlagSetParams = deserialize_params(m).unwrap();
    assert_eq!(out.value, true);
    assert!(out.flag_name.is_none());
}

#[test]
fn deserialize_params_rejects_wrong_types_with_invalid_params_prefix() {
    let mut m = Map::new();
    m.insert(
        "default_temperature".into(),
        Value::String("not-a-number".into()),
    );
    let err = deserialize_params::<ModelSettingsUpdate>(m).unwrap_err();
    assert!(err.starts_with("invalid params"));
}

#[test]
fn deserialize_params_requires_value_on_set_onboarding() {
    let err = deserialize_params::<OnboardingCompletedSetParams>(Map::new()).unwrap_err();
    assert!(err.contains("invalid params"));
}

#[test]
fn deserialize_params_rejects_missing_required_for_set_browser_allow_all() {
    let err = deserialize_params::<SetBrowserAllowAllParams>(Map::new()).unwrap_err();
    assert!(err.contains("invalid params"));
}

#[test]
fn default_onboarding_flag_constant_points_to_hidden_marker() {
    // Keeps the constant's observable value pinned so tool behavior
    // stays stable across refactors.
    assert_eq!(DEFAULT_ONBOARDING_FLAG_NAME, ".skip_onboarding");
}
