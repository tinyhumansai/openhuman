use super::*;

#[test]
fn all_controller_schemas_and_registered_controllers_stay_in_sync() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(schemas.len(), controllers.len());
    assert!(schemas.iter().all(|s| s.namespace == NAMESPACE));
    assert!(controllers.iter().all(|c| c.schema.namespace == NAMESPACE));
}

#[test]
fn unknown_function_schema_returns_error_output() {
    let schema = schemas("not_real");
    assert_eq!(schema.namespace, NAMESPACE);
    assert_eq!(schema.function, "unknown");
    assert_eq!(schema.outputs.len(), 1);
    assert_eq!(schema.outputs[0].name, "error");
}

#[test]
fn ingest_schema_requires_source_kind_source_id_and_payload() {
    let schema = schemas("ingest");
    assert_eq!(schema.function, "ingest");
    let required: Vec<&str> = schema
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"source_kind"));
    assert!(required.contains(&"source_id"));
    assert!(required.contains(&"payload"));
}
