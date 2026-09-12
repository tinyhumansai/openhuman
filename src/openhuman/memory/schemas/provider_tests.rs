use super::*;

#[test]
fn provider_schema_only_exposes_provider_status() {
    assert_eq!(FUNCTIONS, &["provider_status"]);
    assert_eq!(controllers().len(), 1);
}

#[test]
fn unknown_provider_schema_returns_none() {
    assert!(schema("not_real").is_none());
}

#[test]
fn provider_status_schema_has_no_inputs_and_names_the_status_fields() {
    let schema = schema("provider_status").unwrap();
    assert_eq!(schema.namespace, "memory");
    assert!(schema.inputs.is_empty());
    let names: Vec<&str> = schema.outputs.iter().map(|f| f.name).collect();
    for expected in [
        "slot",
        "driver",
        "class",
        "health",
        "contract_version",
        "capabilities",
    ] {
        assert!(names.contains(&expected), "missing output {expected}");
    }
}

#[tokio::test]
async fn handler_returns_driver_and_capability_fields() {
    let value = handle_provider_status(Map::new())
        .await
        .expect("handler succeeds");
    assert!(value["driver"].is_string());
    assert!(value["capabilities"].is_array());
    assert!(value["contract_version"].is_string());
}
