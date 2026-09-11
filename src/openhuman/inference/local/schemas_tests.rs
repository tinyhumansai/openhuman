use super::*;

#[test]
fn catalog_counts_match_and_nonempty() {
    let s = all_controller_schemas();
    let h = all_registered_controllers();
    assert_eq!(s.len(), h.len());
    assert!(
        s.len() >= 10,
        "local inference should expose >=10 controller fns"
    );
}

#[test]
fn all_schemas_use_inference_namespace_and_have_descriptions() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "inference", "function {}", s.function);
        assert!(!s.description.is_empty(), "function {} desc", s.function);
        assert!(!s.outputs.is_empty(), "function {} outputs", s.function);
    }
}

#[test]
fn unknown_function_returns_unknown_schema() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "inference");
}

#[test]
fn every_registered_key_resolves_to_non_unknown_schema() {
    let keys = [
        "agent_chat",
        "agent_chat_simple",
        "transcribe",
        "transcribe_bytes",
        "tts",
        "assets_status",
        "downloads_progress",
        "download_asset",
        "install_piper",
        "piper_install_status",
        "test_connection",
    ];
    for k in keys {
        let s = schemas(k);
        assert_eq!(s.namespace, "inference");
        assert_ne!(s.function, "unknown", "key `{k}` fell through");
    }
}

#[test]
fn registered_controllers_all_in_inference_namespace() {
    for h in all_registered_controllers() {
        assert_eq!(h.schema.namespace, "inference");
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

// ── Handler-level tests that don't need Ollama ────────────────

use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
use tempfile::TempDir;

/// Regression test for the CodeRabbit #7 race on PR #1755: when two concurrent
/// RPC calls (a double-click, or an auto-install firing alongside a manual
/// click) hit `handle_local_ai_install_piper` at the same time, only one must
/// spawn a real install task. The other must short-circuit and return the
/// in-flight status without starting a second download that would race on the
/// same `.part` file.
///
/// Exercises the actual handler — not just the slot primitive — so the wiring
/// at the call site is covered too.
#[tokio::test]
async fn install_piper_handler_serializes_concurrent_calls() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let slot = crate::openhuman::inference::local::voice_install_common::try_acquire_install_slot(
        crate::openhuman::inference::local::voice_install_common::ENGINE_PIPER,
    )
    .expect("test should be able to claim the slot first");

    crate::openhuman::inference::local::voice_install_common::write_status(
        crate::openhuman::inference::local::voice_install_common::VoiceInstallStatus {
            engine: crate::openhuman::inference::local::voice_install_common::ENGINE_PIPER.to_string(),
            state: crate::openhuman::inference::local::voice_install_common::VoiceInstallState::Installing,
            progress: Some(0),
            downloaded_bytes: None,
            total_bytes: None,
            stage: Some("queued".to_string()),
            error_detail: None,
        },
    );

    let (r1, r2) = tokio::join!(
        handle_local_ai_install_piper(Map::new()),
        handle_local_ai_install_piper(Map::new())
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
    drop(slot);
    crate::openhuman::inference::local::voice_install_common::reset_status(
        crate::openhuman::inference::local::voice_install_common::ENGINE_PIPER,
    );

    let v1 = r1.expect("first call ok");
    let v2 = r2.expect("second call ok");
    for (label, v) in [("first", &v1), ("second", &v2)] {
        let state = v.get("state").and_then(|s| s.as_str());
        assert_eq!(
            state,
            Some("installing"),
            "{label} concurrent call should see Installing, got {v:?}"
        );
    }
}
