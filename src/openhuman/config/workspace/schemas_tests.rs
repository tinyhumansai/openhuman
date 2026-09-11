use super::*;

#[test]
fn all_schemas_use_workspace_namespace() {
    let all = all_controller_schemas();
    assert_eq!(all.len(), 3);
    for schema in &all {
        assert_eq!(schema.namespace, "workspace");
    }
}

#[test]
fn registered_controllers_expose_expected_rpc_methods() {
    let methods: Vec<String> = all_registered_controllers()
        .iter()
        .map(|c| c.rpc_method_name())
        .collect();
    assert!(methods.contains(&"openhuman.workspace_file_read".to_string()));
    assert!(methods.contains(&"openhuman.workspace_file_write".to_string()));
    assert!(methods.contains(&"openhuman.workspace_file_reset".to_string()));
}

#[test]
fn file_write_schema_requires_filename_and_contents() {
    let schema = schemas("file_write");
    let input_names: Vec<&str> = schema.inputs.iter().map(|f| f.name).collect();
    assert!(input_names.contains(&"filename"));
    assert!(input_names.contains(&"contents"));
}

#[test]
fn unknown_function_yields_unknown_schema() {
    let schema = schemas("nope");
    assert_eq!(schema.function, "unknown");
}
