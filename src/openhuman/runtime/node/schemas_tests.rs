use super::*;

#[test]
fn catalog_lists_both_runtime_controllers() {
    let schemas = all_controller_schemas();
    assert_eq!(schemas.len(), 2);
    let names: Vec<&str> = schemas.iter().map(|schema| schema.function).collect();
    assert!(names.contains(&"list_tools"));
    assert!(names.contains(&"execute_tool"));
}

#[test]
fn execute_tool_schema_requires_tool_name() {
    let schema = schemas("javascript_execute_tool");
    assert_eq!(schema.namespace, "javascript");
    assert_eq!(schema.function, "execute_tool");
    assert!(schema
        .inputs
        .iter()
        .any(|field| field.name == "tool_name" && field.required));
}
