use super::*;

#[test]
fn all_controller_schemas_and_registered_controllers_stay_in_sync() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(schemas.len(), controllers.len());
    assert!(schemas.iter().all(|schema| schema.namespace == NAMESPACE));
}

#[test]
#[should_panic(expected = "unknown agent_registry schema function")]
fn schemas_panics_on_unknown_function() {
    schemas("missing");
}

#[test]
fn available_tools_schema_is_registered_with_tools_output() {
    let schema = schemas("available_tools");
    assert_eq!(schema.namespace, NAMESPACE);
    assert_eq!(schema.function, "available_tools");
    assert!(schema.inputs.is_empty());
    let tools = schema
        .outputs
        .iter()
        .find(|field| field.name == "tools")
        .expect("available_tools should output a `tools` field");
    assert!(tools.required);
    assert!(all_controller_schemas()
        .iter()
        .any(|s| s.function == "available_tools"));
}
