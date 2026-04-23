use super::*;
use serde_json::json;

// ── Catalog integrity ─────────────────────────────────────────

const EXPECTED_FUNCTIONS: &[&str] = &[
    "list_registrations",
    "list_logs",
    "clear_logs",
    "register_echo",
    "unregister_echo",
    "list_tunnels",
    "create_tunnel",
    "get_tunnel",
    "update_tunnel",
    "delete_tunnel",
    "get_bandwidth",
];

#[test]
fn all_controller_schemas_matches_expected_function_set() {
    let schemas_list = all_controller_schemas();
    assert_eq!(schemas_list.len(), EXPECTED_FUNCTIONS.len());
    let names: Vec<&str> = schemas_list.iter().map(|s| s.function).collect();
    for expected in EXPECTED_FUNCTIONS {
        assert!(
            names.contains(expected),
            "catalog missing `{expected}`; got {names:?}"
        );
    }
}

#[test]
fn all_controller_schemas_entries_are_all_under_webhooks_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(
            s.namespace, "webhooks",
            "schema `{}` has wrong namespace",
            s.function
        );
        assert!(
            !s.description.trim().is_empty(),
            "schema `{}` must have a description",
            s.function
        );
    }
}

#[test]
fn all_registered_controllers_parallels_the_schema_list() {
    let schemas_list = all_controller_schemas();
    let handlers = all_registered_controllers();
    assert_eq!(schemas_list.len(), handlers.len());

    // Every registered controller's schema must resolve back to the
    // same ControllerSchema produced by `schemas()` — proves the two
    // lists are kept in lock-step and no handler is mis-wired.
    for rc in &handlers {
        let resolved = schemas(rc.schema.function);
        assert_eq!(resolved.function, rc.schema.function);
        assert_eq!(resolved.namespace, rc.schema.namespace);
    }
}

#[test]
fn all_registered_controller_function_names_are_unique() {
    let handlers = all_registered_controllers();
    let mut names: Vec<&str> = handlers.iter().map(|rc| rc.schema.function).collect();
    names.sort_unstable();
    let unique_count = {
        let mut clone = names.clone();
        clone.dedup();
        clone.len()
    };
    assert_eq!(
        unique_count,
        names.len(),
        "duplicate function names: {names:?}"
    );
}

// ── schemas(function) per-arm coverage ───────────────────────

fn required_input_names(s: &ControllerSchema) -> Vec<&'static str> {
    s.inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect()
}

#[test]
fn list_registrations_has_no_inputs_and_json_output() {
    let s = schemas("list_registrations");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "result");
    assert!(matches!(s.outputs[0].ty, TypeSchema::Json));
}

#[test]
fn list_logs_limit_is_optional_u64() {
    let s = schemas("list_logs");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "limit");
    assert!(!s.inputs[0].required);
    match &s.inputs[0].ty {
        TypeSchema::Option(inner) => assert!(matches!(**inner, TypeSchema::U64)),
        other => panic!("limit must be Option<U64>, got {other:?}"),
    }
}

#[test]
fn clear_logs_has_no_inputs() {
    assert!(schemas("clear_logs").inputs.is_empty());
}

#[test]
fn register_echo_requires_tunnel_uuid_only() {
    let s = schemas("register_echo");
    assert_eq!(required_input_names(&s), vec!["tunnel_uuid"]);
    // The two optional fields must exist and be Option<String>.
    for optional in ["tunnel_name", "backend_tunnel_id"] {
        let f = s
            .inputs
            .iter()
            .find(|f| f.name == optional)
            .unwrap_or_else(|| panic!("missing optional `{optional}`"));
        assert!(!f.required);
        assert!(
            matches!(&f.ty, TypeSchema::Option(inner) if matches!(**inner, TypeSchema::String))
        );
    }
}

#[test]
fn unregister_echo_requires_tunnel_uuid_only() {
    let s = schemas("unregister_echo");
    assert_eq!(required_input_names(&s), vec!["tunnel_uuid"]);
}

#[test]
fn list_tunnels_has_no_inputs() {
    assert!(schemas("list_tunnels").inputs.is_empty());
}

#[test]
fn create_tunnel_requires_name_and_allows_optional_description() {
    let s = schemas("create_tunnel");
    assert_eq!(required_input_names(&s), vec!["name"]);
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "description" && !f.required));
}

#[test]
fn get_and_delete_tunnel_require_id_only() {
    for fn_name in ["get_tunnel", "delete_tunnel"] {
        let s = schemas(fn_name);
        assert_eq!(
            required_input_names(&s),
            vec!["id"],
            "`{fn_name}` must require only `id`"
        );
    }
}

#[test]
fn update_tunnel_requires_id_and_allows_optional_name_description_is_active() {
    let s = schemas("update_tunnel");
    assert_eq!(required_input_names(&s), vec!["id"]);
    for optional in ["name", "description", "isActive"] {
        assert!(
            s.inputs.iter().any(|f| f.name == optional && !f.required),
            "`update_tunnel` must accept optional `{optional}`"
        );
    }
}

#[test]
fn get_bandwidth_has_no_inputs() {
    assert!(schemas("get_bandwidth").inputs.is_empty());
}

#[test]
fn unknown_function_returns_error_fallback_schema() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "webhooks");
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "error");
    assert!(matches!(s.outputs[0].ty, TypeSchema::String));
    assert!(s.outputs[0].required);
}

// ── deserialize_params ────────────────────────────────────────

#[test]
fn deserialize_params_returns_typed_struct_for_valid_input() {
    let mut params = Map::new();
    params.insert("tunnel_uuid".to_string(), Value::String("u-1".into()));
    params.insert("tunnel_name".to_string(), Value::String("n".into()));
    params.insert("backend_tunnel_id".to_string(), Value::Null);
    let parsed = deserialize_params::<WebhookRegisterEchoParams>(params).unwrap();
    assert_eq!(parsed.tunnel_uuid, "u-1");
    assert_eq!(parsed.tunnel_name.as_deref(), Some("n"));
    assert!(parsed.backend_tunnel_id.is_none());
}

#[test]
fn deserialize_params_reports_invalid_params_errors() {
    // Missing required `tunnel_uuid` for WebhookUnregisterEchoParams.
    let err = deserialize_params::<WebhookUnregisterEchoParams>(Map::new()).unwrap_err();
    assert!(
        err.contains("invalid params"),
        "expected 'invalid params' prefix, got: {err}"
    );
}

#[test]
fn deserialize_params_honours_camel_case_rename_for_update_tunnel() {
    // `WebhookUpdateTunnelParams` uses `#[serde(rename_all = "camelCase")]`,
    // so the JSON key is `isActive` even though the Rust field is
    // `is_active`. This test locks in that contract.
    let mut params = Map::new();
    params.insert("id".to_string(), Value::String("t-1".into()));
    params.insert("isActive".to_string(), Value::Bool(true));
    let parsed = deserialize_params::<WebhookUpdateTunnelParams>(params).unwrap();
    assert_eq!(parsed.id, "t-1");
    assert_eq!(parsed.is_active, Some(true));
}

// ── json_output / to_json ─────────────────────────────────────

#[test]
fn json_output_builds_required_json_field() {
    let f = json_output("result", "stuff");
    assert_eq!(f.name, "result");
    assert_eq!(f.comment, "stuff");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::Json));
}

#[test]
fn to_json_renders_rpc_outcome_in_cli_compatible_shape() {
    // `to_json` is a thin wrapper over `RpcOutcome::into_cli_compatible_json`.
    // We exercise it here so coverage follows the real shape the
    // adapters produce, rather than asserting on implementation details.
    let outcome: RpcOutcome<serde_json::Value> = RpcOutcome::new(json!({"ok": true}), vec![]);
    let value = to_json(outcome).unwrap();
    assert!(value.is_object());
}
