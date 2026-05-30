use super::*;
use serde_json::json;

// ── schemas() coverage ────────────────────────────────────────────────────

#[test]
fn schemas_registry_search_has_no_required_inputs() {
    let s = schemas("registry_search");
    assert_eq!(s.namespace, "mcp_clients");
    assert!(s.inputs.iter().all(|f| !f.required));
}

#[test]
fn schemas_registry_get_requires_qualified_name() {
    let s = schemas("registry_get");
    let qn = s
        .inputs
        .iter()
        .find(|f| f.name == "qualified_name")
        .unwrap();
    assert!(qn.required);
}

#[test]
fn schemas_install_requires_qualified_name_and_env() {
    let s = schemas("install");
    let names: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(names.contains(&"qualified_name"));
    assert!(names.contains(&"env"));
}

#[test]
fn schemas_connect_requires_server_id() {
    let s = schemas("connect");
    let si = s.inputs.iter().find(|f| f.name == "server_id").unwrap();
    assert!(si.required);
}

#[test]
fn schemas_tool_call_requires_three_fields() {
    let s = schemas("tool_call");
    let required: Vec<_> = s.inputs.iter().filter(|f| f.required).collect();
    assert_eq!(required.len(), 3);
}

#[test]
fn schemas_config_assist_history_is_optional() {
    let s = schemas("config_assist");
    let history = s.inputs.iter().find(|f| f.name == "history").unwrap();
    assert!(!history.required);
}

#[test]
fn schemas_unknown_function_returns_placeholder() {
    let s = schemas("not-a-real-function");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

// ── all_controller_schemas / all_registered_controllers ────────────────────

#[test]
fn all_controller_schemas_covers_expected_methods() {
    let schemas = all_controller_schemas();
    // 10 mcp_clients + 6 mcp_setup
    assert_eq!(schemas.len(), 16);
    let mcp_clients_count = schemas
        .iter()
        .filter(|s| s.namespace == "mcp_clients")
        .count();
    let mcp_setup_count = schemas
        .iter()
        .filter(|s| s.namespace == "mcp_setup")
        .count();
    assert_eq!(mcp_clients_count, 10);
    assert_eq!(mcp_setup_count, 6);
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 16);
}

#[test]
fn all_registered_controllers_use_expected_namespaces() {
    for c in all_registered_controllers() {
        assert!(
            matches!(c.schema.namespace, "mcp_clients" | "mcp_setup"),
            "unexpected namespace {}",
            c.schema.namespace
        );
    }
}

// ── read_required ─────────────────────────────────────────────────────────

#[test]
fn read_required_returns_value_for_present_key() {
    let mut params = Map::new();
    params.insert("server_id".into(), json!("srv-1"));
    let got: String = read_required(&params, "server_id").unwrap();
    assert_eq!(got, "srv-1");
}

#[test]
fn read_required_errors_on_missing_key() {
    let err = read_required::<String>(&Map::new(), "server_id").unwrap_err();
    assert!(err.contains("missing required param 'server_id'"));
}

// ── read_optional_u32 ─────────────────────────────────────────────────────

#[test]
fn read_optional_u32_absent_is_none() {
    assert_eq!(read_optional_u32(&Map::new(), "page").unwrap(), None);
}

#[test]
fn read_optional_u32_valid_number() {
    let mut p = Map::new();
    p.insert("page".into(), json!(2));
    assert_eq!(read_optional_u32(&p, "page").unwrap(), Some(2));
}

#[test]
fn read_optional_u32_rejects_negative() {
    let mut p = Map::new();
    p.insert("page".into(), json!(-1));
    assert!(read_optional_u32(&p, "page").is_err());
}

// ── type_name ─────────────────────────────────────────────────────────────

#[test]
fn type_name_covers_all_variants() {
    assert_eq!(type_name(&Value::Null), "null");
    assert_eq!(type_name(&json!(true)), "bool");
    assert_eq!(type_name(&json!(1)), "number");
    assert_eq!(type_name(&json!("s")), "string");
    assert_eq!(type_name(&json!([])), "array");
    assert_eq!(type_name(&json!({})), "object");
}
