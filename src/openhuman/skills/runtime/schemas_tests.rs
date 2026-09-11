use super::*;

#[test]
fn schemas_cover_runtime_cli_surface() {
    let functions: Vec<_> = all_skill_runtime_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect();
    assert_eq!(
        functions,
        vec![
            "run",
            "cancel",
            "recent_runs",
            "read_run_log",
            "resolve_runtimes",
            "schemas"
        ]
    );
    assert!(all_skill_runtime_registered_controllers()
        .iter()
        .all(|controller| controller.schema.namespace == "skill_runtime"));
}

#[test]
fn run_schema_uses_skill_id_not_workflow_id() {
    let schema = skill_runtime_schemas("run");
    assert_eq!(schema.namespace, "skill_runtime");
    assert!(schema.inputs.iter().any(|field| field.name == "skill_id"));
}

#[test]
fn resolve_runtimes_schema_is_cli_friendly() {
    let schema = skill_runtime_schemas("resolve_runtimes");
    assert_eq!(schema.namespace, "skill_runtime");
    assert_eq!(schema.function, "resolve_runtimes");
    assert!(schema.inputs.iter().any(|field| field.name == "runtime"));
}
