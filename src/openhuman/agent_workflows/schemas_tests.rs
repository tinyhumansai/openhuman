//! Branch coverage for the workflows controller schemas.

use super::*;

#[test]
fn all_controller_schemas_covers_every_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names, vec!["list", "read", "create", "uninstall", "phase"]);
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 5);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(names, vec!["list", "read", "create", "uninstall", "phase"]);
}

#[test]
fn every_schema_uses_workflows_namespace() {
    for f in ["list", "read", "create", "uninstall", "phase"] {
        assert_eq!(schemas(f).namespace, "workflows", "fn={f}");
    }
}

#[test]
fn list_outputs_workflow_summary_array() {
    let s = schemas("list");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs[0].name, "workflows");
}

#[test]
fn read_and_phase_require_id() {
    let read = schemas("read");
    assert!(read.inputs.iter().any(|f| f.name == "id" && f.required));

    let phase = schemas("phase");
    let names: Vec<_> = phase.inputs.iter().map(|f| f.name).collect();
    assert!(names.contains(&"id"));
    assert!(names.contains(&"phase"));
}

#[test]
fn create_requires_only_name() {
    let s = schemas("create");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["name"]);
}

#[test]
fn unknown_function_is_placeholder() {
    let s = schemas("does-not-exist");
    assert_eq!(s.function, "unknown");
}
