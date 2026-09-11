use super::*;

#[test]
fn all_controller_schemas_cover_every_registered_retrieval_function() {
    let schemas = all_controller_schemas();
    let functions: Vec<&str> = schemas.iter().map(|s| s.function).collect();
    assert_eq!(
        functions,
        vec![
            "query_source",
            "cover_window",
            "search_entities",
            "drill_down",
            "fetch_leaves",
        ]
    );
}

#[test]
fn registered_controllers_use_memory_tree_namespace() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 5);
    assert!(controllers.iter().all(|c| c.schema.namespace == NAMESPACE));
}

#[test]
fn unknown_schema_returns_error_output() {
    let schema = schemas("not_a_real_function");
    assert_eq!(schema.namespace, NAMESPACE);
    assert_eq!(schema.function, "unknown");
    assert_eq!(schema.outputs.len(), 1);
    assert_eq!(schema.outputs[0].name, "error");
}
