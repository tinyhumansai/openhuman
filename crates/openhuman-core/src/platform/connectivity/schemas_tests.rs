use super::*;
use crate::core::all::assert_schema_controller_parity;

#[test]
fn connectivity_schema_catalog_matches_registered_controllers() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "diag",
    );
}
use crate::core::runtime::DispatchCtx;

#[test]
fn lists_single_diag_controller() {
    let schemas = all_controller_schemas();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].namespace, "connectivity");
    assert_eq!(schemas[0].function, "diag");
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

#[tokio::test]
async fn handle_diag_returns_json_object() {
    let value = handle_diag(DispatchCtx::process_default(), Map::new())
        .await
        .expect("diag handler ok");
    assert!(value.is_object(), "payload should be a JSON object");
}
