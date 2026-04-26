use super::*;

#[test]
fn catalog_counts_match_and_nonempty() {
    let s = all_controller_schemas();
    let h = all_registered_controllers();
    assert_eq!(s.len(), h.len());
    assert!(s.len() >= 20, "local_ai should expose >=20 controller fns");
}

#[test]
fn all_schemas_use_local_ai_namespace_and_have_descriptions() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "local_ai", "function {}", s.function);
        assert!(!s.description.is_empty(), "function {} desc", s.function);
        assert!(!s.outputs.is_empty(), "function {} outputs", s.function);
    }
}

#[test]
fn unknown_function_returns_unknown_schema() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "local_ai");
}

#[test]
fn every_registered_key_resolves_to_non_unknown_schema() {
    let keys = [
        "agent_chat",
        "agent_chat_simple",
        "local_ai_status",
        "local_ai_download",
        "local_ai_download_all_assets",
        "local_ai_summarize",
        "local_ai_prompt",
        "local_ai_vision_prompt",
        "local_ai_embed",
        "local_ai_transcribe",
        "local_ai_transcribe_bytes",
        "local_ai_tts",
        "local_ai_assets_status",
        "local_ai_downloads_progress",
        "local_ai_download_asset",
        "local_ai_device_profile",
        "local_ai_presets",
        "local_ai_apply_preset",
        "local_ai_set_ollama_path",
        "local_ai_diagnostics",
        "local_ai_chat",
        "local_ai_should_react",
        "local_ai_analyze_sentiment",
        "local_ai_should_send_gif",
        "local_ai_tenor_search",
    ];
    for k in keys {
        let s = schemas(k);
        assert_eq!(s.namespace, "local_ai");
        assert_ne!(s.function, "unknown", "key `{k}` fell through");
    }
}

#[test]
fn registered_controllers_all_in_local_ai_namespace() {
    for h in all_registered_controllers() {
        assert_eq!(h.schema.namespace, "local_ai");
        assert!(!h.schema.function.is_empty());
    }
}

#[test]
fn field_builder_helpers_are_correct_shape() {
    let r = required_string("k", "c");
    assert!(r.required);
    assert!(matches!(r.ty, TypeSchema::String));

    let o = optional_string("k", "c");
    assert!(!o.required);

    let ou = optional_u64("k", "c");
    assert!(!ou.required);

    let j = json_output("result", "c");
    assert!(j.required);
    assert!(matches!(j.ty, TypeSchema::Json));
}

#[test]
fn to_json_wraps_rpc_outcome() {
    let v =
        to_json(RpcOutcome::single_log(serde_json::json!({"ok": true}), "l")).expect("serialize");
    assert!(v.get("logs").is_some() || v.get("result").is_some() || v.get("ok").is_some());
}

#[test]
fn deserialize_params_parses_valid_object() {
    let mut m = Map::new();
    m.insert("message".into(), Value::String("hi".into()));
    let p: AgentChatParams = deserialize_params(m).expect("parse");
    assert_eq!(p.message, "hi");
}

#[test]
fn deserialize_params_errors_on_invalid_shape() {
    let mut m = Map::new();
    m.insert("message".into(), Value::Bool(true));
    let err = deserialize_params::<AgentChatParams>(m).unwrap_err();
    assert!(err.contains("invalid params"));
}

#[test]
fn prompt_schema_has_inputs() {
    let s = schemas("local_ai_prompt");
    assert!(!s.inputs.is_empty());
}

#[test]
fn apply_preset_schema_has_inputs() {
    let s = schemas("local_ai_apply_preset");
    assert!(!s.inputs.is_empty());
}

#[test]
fn download_schema_optional_force_flag() {
    let s = schemas("local_ai_download");
    let force = s.inputs.iter().find(|f| f.name == "force");
    assert!(force.is_some_and(|f| !f.required));
}

#[test]
fn summarize_schema_requires_text_or_equivalent() {
    let s = schemas("local_ai_summarize");
    assert!(s.inputs.iter().any(|f| f.required));
}

// ── Handler-level tests that don't need Ollama ────────────────

use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
use tempfile::TempDir;

#[tokio::test]
async fn handle_device_profile_returns_device_shape() {
    let v = handle_local_ai_device_profile(Map::new())
        .await
        .expect("ok");
    // device profile exposes at least a few expected fields.
    assert!(v.is_object());
}

#[tokio::test]
async fn handle_presets_returns_presets_list_and_recommended_tier() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let v = handle_local_ai_presets(Map::new()).await.expect("ok");
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
    assert!(v.get("presets").is_some());
    assert!(v.get("recommended_tier").is_some());
    assert!(v.get("device").is_some());
}

#[tokio::test]
async fn handle_apply_preset_rejects_invalid_tier() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let params = Map::from_iter([("tier".to_string(), serde_json::json!("ram_bogus"))]);
    let err = handle_local_ai_apply_preset(params).await.unwrap_err();
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
    assert!(err.contains("invalid tier"));
}

#[tokio::test]
async fn handle_apply_preset_rejects_custom_tier() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let params = Map::from_iter([("tier".to_string(), serde_json::json!("custom"))]);
    let err = handle_local_ai_apply_preset(params).await.unwrap_err();
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
    assert!(err.contains("cannot apply 'custom'"));
}

#[tokio::test]
async fn handle_apply_preset_accepts_valid_tier_and_persists() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let params = Map::from_iter([("tier".to_string(), serde_json::json!("ram_2_4gb"))]);
    let result = handle_local_ai_apply_preset(params)
        .await
        .expect("apply ok");
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
    assert!(result.get("applied_tier").is_some());
    assert!(result.get("chat_model_id").is_some());
}

#[tokio::test]
async fn handle_set_ollama_path_rejects_nonexistent_path() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let params = Map::from_iter([(
        "path".to_string(),
        serde_json::json!("/this/path/should/not/exist/ollama"),
    )]);
    let err = handle_local_ai_set_ollama_path(params).await.unwrap_err();
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
    assert!(err.contains("Ollama binary not found"));
}

#[tokio::test]
async fn handle_set_ollama_path_accepts_empty_string_to_clear() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let params = Map::from_iter([("path".to_string(), serde_json::json!(""))]);
    // Empty path clears the setting — must not error.
    let _ = handle_local_ai_set_ollama_path(params).await.expect("ok");
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}
