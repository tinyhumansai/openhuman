use super::*;

#[test]
fn schema_inventory_matches_handlers() {
    assert_eq!(
        all_controller_schemas().len(),
        all_registered_controllers().len()
    );
}

#[test]
fn start_schema_requires_directory_and_port() {
    let schema = schemas("start");
    let required: Vec<&str> = schema
        .inputs
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    assert_eq!(required, vec!["directory", "port"]);
}

#[test]
fn stop_schema_requires_server_id() {
    let schema = schemas("stop");
    assert_eq!(schema.inputs.len(), 1);
    assert_eq!(schema.inputs[0].name, "server_id");
    assert!(schema.inputs[0].required);
}

#[test]
fn unknown_schema_falls_back() {
    let schema = schemas("nope");
    assert_eq!(schema.namespace, "http_host");
    assert_eq!(schema.function, "unknown");
}
