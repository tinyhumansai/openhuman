use super::*;
use crate::agent_runs::agent_chat::params::AgentChatParams;
use crate::core::all::assert_schema_controller_parity;

#[test]
fn runtime_schema_catalog_matches_registered_controllers() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "inference");
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "transcribe",
    );
}

#[test]
fn every_registered_key_resolves_to_non_unknown_schema() {
    let keys = ["transcribe", "transcribe_bytes", "tts", "test_connection"];
    for k in keys {
        let s = schemas(k);
        assert_eq!(s.namespace, "inference");
        assert_ne!(s.function, "unknown", "key `{k}` fell through");
    }
}

/// Model download, asset, and Piper-install controllers were removed: the
/// user runs their own local runtime and pulls models there.
#[test]
fn download_and_installer_controllers_are_not_registered() {
    let functions: Vec<&str> = all_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect();
    for removed in [
        "assets_status",
        "downloads_progress",
        "download_asset",
        "install_piper",
        "piper_install_status",
    ] {
        assert!(
            !functions.contains(&removed),
            "`{removed}` still registered"
        );
        assert_eq!(schemas(removed).function, "unknown");
    }
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

    let j = json_output("result", "c");
    assert!(j.required);
    assert!(matches!(j.ty, TypeSchema::Json));
}

#[test]
fn to_json_wraps_rpc_outcome() {
    let v = to_json(Outcome::single_log(serde_json::json!({"ok": true}), "l")).expect("serialize");
    assert!(v.get("logs").is_some() || v.get("result").is_some() || v.get("ok").is_some());
}
