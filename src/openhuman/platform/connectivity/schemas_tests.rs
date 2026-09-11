use super::*;

#[test]
fn lists_single_diag_controller() {
    let schemas = all_controller_schemas();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].namespace, "connectivity");
    assert_eq!(schemas[0].function, "diag");
}

#[test]
fn registered_count_matches_schema_count() {
    assert_eq!(
        all_controller_schemas().len(),
        all_registered_controllers().len()
    );
}

#[test]
fn diag_schema_has_no_inputs() {
    assert!(schemas("diag").inputs.is_empty());
}

#[test]
fn diag_schema_outputs_a_diag_payload_field() {
    let s = schemas("diag");
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "diag");
}

#[test]
fn unknown_function_returns_unknown_fallback() {
    let s = schemas("no_such");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "connectivity");
}

#[tokio::test]
async fn handle_diag_returns_json_object() {
    let value = handle_diag(Map::new()).await.expect("diag handler ok");
    assert!(value.is_object(), "payload should be a JSON object");
}
