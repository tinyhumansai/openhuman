use super::*;
use serde_json::json;

#[test]
fn catalog_counts_match() {
    let s = all_controller_schemas();
    let h = all_registered_controllers();
    assert_eq!(s.len(), h.len());
    assert!(s.len() >= 9);
}

#[test]
fn all_schemas_use_composio_namespace_and_have_descriptions() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "composio", "function {}", s.function);
        assert!(!s.description.is_empty());
        assert!(
            !s.outputs.is_empty(),
            "function {} has no outputs",
            s.function
        );
    }
}

#[test]
fn every_known_schema_key_resolves() {
    let keys = [
        "list_toolkits",
        "list_connections",
        "authorize",
        "delete_connection",
        "list_tools",
        "execute",
        "get_user_profile",
        "sync",
        "list_trigger_history",
        "get_user_scopes",
        "set_user_scopes",
    ];
    for k in keys {
        let s = schemas(k);
        assert_eq!(s.namespace, "composio");
        assert_ne!(s.function, "unknown", "key `{k}` fell through");
    }
}

#[test]
fn unknown_function_returns_unknown_schema() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "function");
}

#[test]
fn authorize_schema_requires_toolkit() {
    let s = schemas("authorize");
    let tk = s.inputs.iter().find(|f| f.name == "toolkit").unwrap();
    assert!(tk.required);
}

#[test]
fn execute_schema_requires_tool_and_accepts_optional_arguments() {
    let s = schemas("execute");
    assert!(s.inputs.iter().any(|f| f.name == "tool" && f.required));
    let args = s.inputs.iter().find(|f| f.name == "arguments");
    assert!(args.is_some());
    assert!(!args.unwrap().required);
}

#[test]
fn sync_schema_requires_connection_id_and_optional_reason() {
    let s = schemas("sync");
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "connection_id" && f.required));
    let reason = s.inputs.iter().find(|f| f.name == "reason");
    assert!(reason.is_some_and(|f| !f.required));
}

// ── read_required / read_required_non_empty / read_optional ────

#[test]
fn read_required_parses_string_value() {
    let mut m = Map::new();
    m.insert("toolkit".into(), Value::String("gmail".into()));
    let v: String = read_required(&m, "toolkit").unwrap();
    assert_eq!(v, "gmail");
}

#[test]
fn read_required_errors_when_missing() {
    let m = Map::new();
    let err = read_required::<String>(&m, "toolkit").unwrap_err();
    assert!(err.contains("missing required param"));
}

#[test]
fn read_required_errors_when_wrong_type() {
    let mut m = Map::new();
    m.insert("toolkit".into(), json!(42));
    let err = read_required::<String>(&m, "toolkit").unwrap_err();
    assert!(err.contains("invalid 'toolkit'"));
}

#[test]
fn read_required_non_empty_rejects_blank_and_whitespace() {
    let mut m = Map::new();
    m.insert("toolkit".into(), Value::String("".into()));
    assert!(read_required_non_empty(&m, "toolkit")
        .unwrap_err()
        .contains("must not be empty"));
    m.insert("toolkit".into(), Value::String("   ".into()));
    assert!(read_required_non_empty(&m, "toolkit")
        .unwrap_err()
        .contains("must not be empty"));
}

#[test]
fn read_required_non_empty_trims_value() {
    let mut m = Map::new();
    m.insert("toolkit".into(), Value::String("  gmail ".into()));
    assert_eq!(read_required_non_empty(&m, "toolkit").unwrap(), "gmail");
}

#[test]
fn read_optional_returns_none_on_missing_or_null() {
    let mut m = Map::new();
    assert_eq!(read_optional::<String>(&m, "k").unwrap(), None);
    m.insert("k".into(), Value::Null);
    assert_eq!(read_optional::<String>(&m, "k").unwrap(), None);
}

#[test]
fn read_optional_parses_typed_value() {
    let mut m = Map::new();
    m.insert("toolkits".into(), json!(["gmail", "notion"]));
    let v: Vec<String> = read_optional(&m, "toolkits").unwrap().unwrap();
    assert_eq!(v, vec!["gmail".to_string(), "notion".to_string()]);
}

#[test]
fn read_optional_errors_on_type_mismatch() {
    let mut m = Map::new();
    m.insert("toolkits".into(), Value::String("not-an-array".into()));
    let err = read_optional::<Vec<String>>(&m, "toolkits").unwrap_err();
    assert!(err.contains("invalid 'toolkits'"));
}

#[test]
fn to_json_wraps_outcome() {
    let v = to_json(RpcOutcome::single_log(json!({"x": 1}), "note")).unwrap();
    assert!(v.get("logs").is_some() || v.get("result").is_some() || v.get("x").is_some());
}
