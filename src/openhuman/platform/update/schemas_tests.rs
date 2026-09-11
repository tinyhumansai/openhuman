use super::*;

#[test]
fn all_schemas_returns_four() {
    assert_eq!(all_controller_schemas().len(), 4);
}

#[test]
fn all_controllers_returns_four() {
    assert_eq!(all_registered_controllers().len(), 4);
}

#[test]
fn version_schema_has_no_inputs() {
    let s = schemas("version");
    assert_eq!(s.namespace, "update");
    assert_eq!(s.function, "version");
    assert!(s.inputs.is_empty());
    assert!(!s.outputs.is_empty());
}

#[test]
fn run_schema_has_no_inputs() {
    let s = schemas("run");
    assert_eq!(s.namespace, "update");
    assert_eq!(s.function, "run");
    assert!(s.inputs.is_empty());
    assert!(!s.outputs.is_empty());
}

#[test]
fn check_schema() {
    let s = schemas("check");
    assert_eq!(s.namespace, "update");
    assert_eq!(s.function, "check");
    assert!(s.inputs.is_empty());
    assert!(!s.outputs.is_empty());
}

#[test]
fn apply_schema_requires_download_url_and_asset_name() {
    let s = schemas("apply");
    assert_eq!(s.function, "apply");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"download_url"));
    assert!(required.contains(&"asset_name"));
}

#[test]
fn apply_schema_has_optional_staging_dir() {
    let s = schemas("apply");
    let staging = s.inputs.iter().find(|f| f.name == "staging_dir");
    assert!(staging.is_some_and(|f| !f.required));
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("nonexistent");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "update");
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}
