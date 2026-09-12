use super::*;
use serde_json::json;

#[test]
fn schemas_create_pairing_has_correct_shape() {
    let s = schemas("create_pairing");
    assert_eq!(s.namespace, "devices");
    assert_eq!(s.function, "create_pairing");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "label");
    assert!(!s.inputs[0].required);
    assert!(s.outputs.iter().any(|f| f.name == "channel_id"));
    assert!(s.outputs.iter().any(|f| f.name == "pairing_token"));
    assert!(s.outputs.iter().any(|f| f.name == "core_pubkey"));
}

#[test]
fn schemas_list_has_no_inputs_and_devices_output() {
    let s = schemas("list");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "devices");
}

#[test]
fn schemas_revoke_requires_channel_id() {
    let s = schemas("revoke");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "channel_id");
    assert!(s.inputs[0].required);
    assert_eq!(s.outputs[0].name, "success");
}

#[test]
fn schemas_unknown_returns_error_placeholder() {
    let s = schemas("does-not-exist");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

#[test]
fn all_controller_schemas_covers_three_functions() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names, vec!["create_pairing", "list", "revoke"]);
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 3);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(names, vec!["create_pairing", "list", "revoke"]);
}

#[test]
fn read_required_errors_when_key_missing() {
    let params = Map::new();
    let err = read_required::<String>(&params, "channel_id").unwrap_err();
    assert!(err.contains("missing required param 'channel_id'"));
}

#[test]
fn read_optional_string_absent_key_is_none() {
    let result = read_optional_string(&Map::new(), "label").unwrap();
    assert!(result.is_none());
}

#[test]
fn read_optional_string_present_value_returned() {
    let mut params = Map::new();
    params.insert("label".into(), json!("iPhone 15"));
    let result = read_optional_string(&params, "label").unwrap();
    assert_eq!(result, Some("iPhone 15".to_string()));
}

#[test]
fn type_name_covers_all_variants() {
    assert_eq!(type_name(&Value::Null), "null");
    assert_eq!(type_name(&json!(true)), "bool");
    assert_eq!(type_name(&json!(1)), "number");
    assert_eq!(type_name(&json!("s")), "string");
    assert_eq!(type_name(&json!([])), "array");
    assert_eq!(type_name(&json!({})), "object");
}
