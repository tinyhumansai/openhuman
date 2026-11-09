use super::*;
use serde_json::json;

use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_input_order, expect_schema_output_names, expect_schema_required_inputs,
};
use serde_json::Map;

#[test]
fn registry_schema_contracts_are_preserved() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "mcp_clients");
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "config_get",
    );

    let s = schemas("registry_search");
    let catalog = all_controller_schemas();
    assert_eq!(catalog.len(), 17);
    let functions: Vec<_> = catalog.iter().map(|s| s.function).collect();
    for name in [
        "config_get",
        "config_set",
        "list_tools",
        "update_env",
        "registry_settings_get",
        "registry_settings_set",
        "set_enabled",
        "detect_auth",
        "oauth_begin",
    ] {
        assert!(functions.contains(&name), "missing mcp_clients.{name}");
    }
    assert!(!functions.contains(&"install"));
    assert!(!functions.contains(&"config_assist"));
    assert_eq!(s.namespace, "mcp_clients");
    expect_schema_required_inputs(&s, &[]);
    expect_schema_identity(&schemas("registry_get"), "mcp_clients", "registry_get");
    expect_schema_fields(&schemas("registry_get"), &[("qualified_name", Some(true))]);

    let set = schemas("config_set");
    expect_schema_identity(&set, "mcp_clients", "config_set");
    expect_schema_input_order(&set, &["mcpServers"]);
    expect_schema_required_inputs(&set, &["mcpServers"]);
    expect_schema_output_names(&set, &["mcpServers", "added", "updated", "removed"]);

    let get = schemas("config_get");
    assert!(get.inputs.is_empty());
    assert_eq!(get.outputs[0].name, "mcpServers");
    expect_schema_fields(&schemas("connect"), &[("server_id", Some(true))]);
    assert_eq!(
        schemas("tool_call")
            .inputs
            .iter()
            .filter(|field| field.required)
            .count(),
        3
    );
    assert_eq!(schemas("install").function, "unknown");
    assert_eq!(schemas("config_assist").function, "unknown");
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
fn registry_get_shared_parser_preserves_value_and_validation_errors() {
    let valid = Map::from_iter([("qualified_name".into(), json!("example/server"))]);
    assert_eq!(
        super::handlers::parse_registry_get(&valid).unwrap(),
        "example/server"
    );
    let padded = Map::from_iter([("qualified_name".into(), json!(" example/server "))]);
    assert_eq!(
        super::handlers::parse_registry_get(&padded).unwrap(),
        " example/server ",
        "registry parser preserves the qualified name for downstream operation validation"
    );
    assert!(super::handlers::parse_registry_get(&Map::new())
        .unwrap_err()
        .contains("missing required param 'qualified_name'"));
    let invalid = Map::from_iter([("qualified_name".into(), json!(42))]);
    assert!(super::handlers::parse_registry_get(&invalid)
        .unwrap_err()
        .contains("invalid 'qualified_name'"));
}

#[test]
fn registry_get_legacy_catalog_keeps_its_handler_and_fixture_contract() {
    let controllers = all_registered_controllers()
        .into_iter()
        .filter(|controller| controller.schema.method_name() == "mcp_clients.registry_get")
        .collect::<Vec<_>>();
    assert_eq!(controllers.len(), 1);
    assert!(std::ptr::fn_addr_eq(
        controllers[0].handler,
        super::handlers::handle_registry_get as crate::core::all::ControllerHandler
    ));
    crate::core::all::verify_parameter_schema(
        &controllers[0].schema,
        &super::registry_get_parameter_contract(),
    )
    .expect("legacy catalogue keeps its authored schema example");
}
