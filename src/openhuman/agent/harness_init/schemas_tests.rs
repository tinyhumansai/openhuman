use super::*;

#[test]
fn status_has_no_inputs_and_snapshot_output() {
    let s = schemas("status");
    assert_eq!(s.namespace, "harness_init");
    assert_eq!(s.function, "status");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "snapshot");
}

#[test]
fn run_has_optional_force_input() {
    let s = schemas("run");
    let force = s.inputs.iter().find(|f| f.name == "force").unwrap();
    assert!(!force.required);
}

#[test]
fn unknown_function_returns_placeholder_with_error_output() {
    let s = schemas("does-not-exist");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

#[test]
fn all_controller_schemas_covers_supported_functions() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names, vec!["status", "run"]);
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 2);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(names, vec!["status", "run"]);
}
