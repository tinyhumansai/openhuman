use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn status_schema_shape() {
    let schema = schemas("status");
    assert_eq!(schema.namespace, "subsystems");
    assert_eq!(schema.function, "status");
    assert!(schema.inputs.is_empty());
    assert_eq!(schema.outputs.len(), 1);
    assert_eq!(schema.outputs[0].name, "subsystems");
}

#[test]
fn schemas_and_controllers_line_up() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(schemas.len(), controllers.len());
    for (schema, controller) in schemas.iter().zip(controllers.iter()) {
        assert_eq!(schema.namespace, controller.schema.namespace);
        assert_eq!(schema.function, controller.schema.function);
    }
}

#[tokio::test]
async fn handler_returns_a_subsystems_array_containing_the_memory_slot() {
    let value = handle_status(DispatchCtx::process_default(), Map::new())
        .await
        .expect("handler succeeds");
    let rows = value["subsystems"].as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slot"], "memory");
    assert!(rows[0]["contract_version"].is_string());
    assert!(rows[0]["capabilities"].is_array());
}
