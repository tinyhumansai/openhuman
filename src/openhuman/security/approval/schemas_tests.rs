use super::*;
use serde_json::json;

#[test]
fn schemas_list_pending_has_no_inputs() {
    let s = schemas("list_pending");
    assert_eq!(s.namespace, "approval");
    assert_eq!(s.function, "list_pending");
    assert!(s.inputs.is_empty());
}

#[test]
fn schemas_decide_requires_request_id_and_decision() {
    let s = schemas("decide");
    let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert!(names.contains(&"request_id"));
    assert!(names.contains(&"decision"));
    assert!(s.inputs.iter().all(|f| f.required));
}

#[test]
fn schemas_unknown_returns_placeholder() {
    let s = schemas("nope");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 5);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(
        names,
        vec![
            "list_pending",
            "list_recent_decisions",
            "decide",
            "get_gate_state",
            "preauthorize_flow",
        ]
    );
}

#[test]
fn schemas_preauthorize_flow_requires_flow_id_and_tool_names() {
    let s = schemas("preauthorize_flow");
    assert_eq!(s.namespace, "approval");
    let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["flow_id", "tool_names"]);
    assert!(s.inputs.iter().all(|f| f.required));
}

#[test]
fn schemas_list_recent_decisions_has_optional_limit() {
    let s = schemas("list_recent_decisions");
    assert_eq!(s.namespace, "approval");
    assert_eq!(s.function, "list_recent_decisions");
    assert_eq!(s.inputs[0].name, "limit");
    assert!(!s.inputs[0].required);
}

#[test]
fn read_required_string_returns_value_for_present_key() {
    let mut params = Map::new();
    params.insert("request_id".into(), json!("abc"));
    let got = read_required_string(&params, "request_id").unwrap();
    assert_eq!(got, "abc");
}

#[test]
fn read_required_string_rejects_wrong_type() {
    let mut params = Map::new();
    params.insert("decision".into(), json!(42));
    let err = read_required_string(&params, "decision").unwrap_err();
    assert!(err.contains("expected string"));
}

#[test]
fn read_required_string_missing_key_errors() {
    let err = read_required_string(&Map::new(), "request_id").unwrap_err();
    assert!(err.contains("missing required"));
}

#[test]
fn read_optional_u64_accepts_missing_and_number() {
    assert_eq!(read_optional_u64(&Map::new(), "limit").unwrap(), None);
    let mut params = Map::new();
    params.insert("limit".into(), json!(25));
    assert_eq!(read_optional_u64(&params, "limit").unwrap(), Some(25));
}
