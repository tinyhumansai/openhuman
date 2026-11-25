use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
};
use serde_json::json;

#[test]
fn composio_schema_contracts_and_registration_are_preserved() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "composio");
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "execute",
    );
    let keys = [
        "list_toolkits",
        "list_capabilities",
        "list_connections",
        "authorize",
        "delete_connection",
        "list_tools",
        "execute",
        "get_user_profile",
        "list_trigger_history",
        "get_user_scopes",
        "set_user_scopes",
        "list_available_triggers",
        "list_triggers",
        "enable_trigger",
        "disable_trigger",
    ];
    for function in keys {
        expect_schema_identity(&schemas(function), "composio", function);
    }
    let registered = all_registered_controllers();
    for function in [
        "list_available_triggers",
        "list_triggers",
        "enable_trigger",
        "disable_trigger",
        "list_trigger_history",
    ] {
        assert!(
            registered.iter().any(|controller| {
                controller.schema.namespace == "composio" && controller.schema.function == function
            }),
            "composio.{function} is not registered"
        );
    }
    let capabilities = schemas("list_capabilities");
    assert!(capabilities.inputs.is_empty());
    assert!(capabilities
        .outputs
        .iter()
        .any(|field| field.name == "capabilities" && field.required));
    expect_schema_fields(&schemas("authorize"), &[("toolkit", Some(true))]);
    expect_schema_fields(
        &schemas("execute"),
        &[("tool", Some(true)), ("arguments", Some(false))],
    );

    expect_schema_fields(
        &schemas("list_available_triggers"),
        &[("toolkit", Some(true)), ("connection_id", Some(false))],
    );
    expect_schema_fields(&schemas("list_triggers"), &[("toolkit", Some(false))]);
    expect_schema_fields(
        &schemas("enable_trigger"),
        &[
            ("connection_id", Some(true)),
            ("slug", Some(true)),
            ("trigger_config", Some(false)),
        ],
    );
    expect_schema_fields(&schemas("disable_trigger"), &[("trigger_id", Some(true))]);
}

// ── read_required / read_required_non_empty / read_optional ────

#[test]
fn read_required_and_non_empty_helpers_preserve_errors_and_trimming() {
    let mut m = Map::new();
    m.insert("toolkit".into(), Value::String("gmail".into()));
    let v: String = read_required(&m, "toolkit").unwrap();
    assert_eq!(v, "gmail");
    assert!(read_required::<String>(&Map::new(), "toolkit")
        .unwrap_err()
        .contains("missing required param"));
    let wrong = Map::from_iter([("toolkit".into(), json!(42))]);
    assert!(read_required::<String>(&wrong, "toolkit")
        .unwrap_err()
        .contains("invalid 'toolkit'"));
    let mut m = Map::new();
    for blank in ["", "   "] {
        m.insert("toolkit".into(), Value::String(blank.into()));
        assert!(read_required_non_empty(&m, "toolkit")
            .unwrap_err()
            .contains("must not be empty"));
    }
    m.insert("toolkit".into(), Value::String("  gmail ".into()));
    assert_eq!(read_required_non_empty(&m, "toolkit").unwrap(), "gmail");
}

#[test]
fn read_optional_helpers_preserve_typed_values_and_errors() {
    let mut m = Map::new();
    assert_eq!(read_optional::<String>(&m, "k").unwrap(), None);
    m.insert("k".into(), Value::Null);
    assert_eq!(read_optional::<String>(&m, "k").unwrap(), None);
    m.insert("toolkits".into(), json!(["gmail", "notion"]));
    let v: Vec<String> = read_optional(&m, "toolkits").unwrap().unwrap();
    assert_eq!(v, vec!["gmail".to_string(), "notion".to_string()]);
    m.insert("toolkits".into(), Value::String("not-an-array".into()));
    let err = read_optional::<Vec<String>>(&m, "toolkits").unwrap_err();
    assert!(err.contains("invalid 'toolkits'"));
}

#[test]
fn controller_schema_catalog_covers_each_declared_function_shape() {
    // Pin required inputs rather than exact counts so additive optional params
    // remain backwards-compatible. This was previously checked by a host
    // reachability suite despite being a schema contract owned by this domain.
    let expected: [(&str, &[&str], &str); 22] = [
        ("list_toolkits", &[], "toolkits"),
        ("list_capabilities", &[], "capabilities"),
        ("list_agent_ready_toolkits", &[], "toolkits"),
        ("list_connections", &[], "connections"),
        ("authorize", &["toolkit"], "connectUrl"),
        ("delete_connection", &["connection_id"], "deleted"),
        ("list_tools", &["toolkits"], "tools"),
        ("execute", &["tool", "connection_id"], "result"),
        ("list_github_repos", &["connection_id"], "result"),
        ("create_trigger", &["slug", "connection_id"], "result"),
        ("get_user_profile", &["connection_id"], "profile"),
        ("refresh_all_identities", &[], "report"),
        ("list_trigger_history", &["limit"], "result"),
        ("get_user_scopes", &["toolkit"], "pref"),
        ("set_user_scopes", &["toolkit", "read", "write"], "pref"),
        ("list_available_triggers", &["toolkit"], "triggers"),
        ("list_triggers", &["toolkit"], "triggers"),
        ("enable_trigger", &["connection_id", "slug"], "result"),
        ("disable_trigger", &["trigger_id"], "deleted"),
        ("get_mode", &[], "mode"),
        ("set_api_key", &["api_key"], "result"),
        ("clear_api_key", &[], "result"),
    ];

    for (function, required_inputs, first_output) in expected {
        let schema = schemas(function);
        assert_eq!(schema.namespace, "composio");
        assert_eq!(schema.function, function);
        let input_names: Vec<&str> = schema.inputs.iter().map(|field| field.name).collect();
        for required in required_inputs {
            assert!(
                input_names.contains(required),
                "{function} must declare input `{required}` (got {input_names:?})"
            );
        }
        assert_eq!(schema.outputs[0].name, first_output, "{function}");
        assert!(!schema.description.is_empty());
    }
}
