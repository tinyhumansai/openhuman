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
        "update_local_ai_settings",
        "resolve_api_url",
        "get_runtime_flags",
        "set_browser_allow_all",
        "workspace_onboarding_flag_exists",
        "workspace_onboarding_flag_set",
        "update_analytics_settings",
        "get_analytics_settings",
        "update_meet_settings",
        "get_meet_settings",
        "update_autonomy_settings",
        "get_autonomy_settings",
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
fn deserialize_params_parses_autonomy_update_with_trusted_roots() {
    // Mirrors the JSON the AgentAccessPanel posts.
    let params = serde_json::json!({
        "level": "supervised",
        "workspace_only": true,
        "allow_tool_install": false,
        "trusted_roots": [
            { "path": "/data/repo", "access": "readwrite" },
            { "path": "/srv/docs" }
        ]
    });
    let m = params.as_object().unwrap().clone();
    let out: AutonomySettingsUpdate = deserialize_params(m).unwrap();
    assert_eq!(out.level.as_deref(), Some("supervised"));
    assert_eq!(out.workspace_only, Some(true));
    assert_eq!(out.allow_tool_install, Some(false));
    let roots = out.trusted_roots.expect("trusted_roots present");
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].path, "/data/repo");
    assert_eq!(
        roots[0].access,
        crate::openhuman::security::TrustedAccess::ReadWrite
    );
    // `access` defaults to Read when omitted.
    assert_eq!(
        roots[1].access,
        crate::openhuman::security::TrustedAccess::Read
    );
}

#[test]
fn autonomy_settings_rpc_is_registered() {
    let funcs: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.function)
        .collect();
    assert!(funcs.contains(&"get_autonomy_settings"));
    assert!(funcs.contains(&"update_autonomy_settings"));
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
fn deserialize_params_parses_local_ai_settings_update() {
    let mut m = Map::new();
    m.insert("runtime_enabled".into(), Value::Bool(true));
    m.insert("opt_in_confirmed".into(), Value::Bool(true));
    m.insert("provider".into(), Value::String("lm_studio".into()));
    m.insert(
        "base_url".into(),
        Value::String("http://localhost:1234/v1".into()),
    );
    m.insert("model_id".into(), Value::String("local-default".into()));
    m.insert("chat_model_id".into(), Value::String("local-chat".into()));
    m.insert("usage_embeddings".into(), Value::Bool(true));
    m.insert("usage_subconscious".into(), Value::Bool(false));

    let out: LocalAiSettingsUpdate = deserialize_params(m).unwrap();
    assert_eq!(out.runtime_enabled, Some(true));
    assert_eq!(out.opt_in_confirmed, Some(true));
    assert_eq!(out.provider.as_deref(), Some("lm_studio"));
    assert_eq!(out.base_url.as_deref(), Some("http://localhost:1234/v1"));
    assert_eq!(out.model_id.as_deref(), Some("local-default"));
    assert_eq!(out.chat_model_id.as_deref(), Some("local-chat"));
    assert_eq!(out.usage_embeddings, Some(true));
    assert_eq!(out.usage_subconscious, Some(false));
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

// ── autonomy settings handlers ───────────────────────────────

use crate::openhuman::config::TEST_ENV_LOCK;

#[tokio::test]
async fn handle_get_autonomy_settings_returns_current_value() {
    let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    // Seed a known value before reading.
    let _ = crate::openhuman::config::ops::load_and_apply_autonomy_settings(
        crate::openhuman::config::ops::AutonomySettingsPatch {
            max_actions_per_hour: Some(123),
            ..Default::default()
        },
    )
    .await
    .expect("seed");

    let out = super::handle_get_autonomy_settings(serde_json::Map::new())
        .await
        .expect("handler");
    // into_cli_compatible_json wraps data under "result" when logs are present.
    let inner = out.get("result").unwrap_or(&out);
    let value = inner.get("max_actions_per_hour").and_then(|v| v.as_u64());
    assert_eq!(value, Some(123));

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn handle_update_autonomy_settings_rejects_invalid_value() {
    let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let mut params = serde_json::Map::new();
    params.insert("max_actions_per_hour".into(), serde_json::json!(0));

    let err = super::handle_update_autonomy_settings(params)
        .await
        .unwrap_err();
    assert!(err.contains("at least 1"), "got: {err}");

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}
