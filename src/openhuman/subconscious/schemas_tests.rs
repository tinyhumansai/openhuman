use super::*;

#[test]
fn all_schemas_returns_ten() {
    assert_eq!(all_controller_schemas().len(), 10);
}

#[test]
fn all_controllers_returns_ten() {
    assert_eq!(all_registered_controllers().len(), 10);
}

#[test]
fn all_use_subconscious_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "subconscious");
        assert!(!s.description.is_empty());
    }
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}

#[test]
fn known_functions_resolve() {
    for fn_name in [
        "status",
        "trigger",
        "tasks_list",
        "tasks_add",
        "tasks_update",
        "tasks_remove",
        "log_list",
        "escalations_list",
        "escalations_approve",
        "escalations_dismiss",
    ] {
        let s = schemas(fn_name);
        assert_ne!(s.function, "unknown", "{fn_name} fell through");
    }
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("nonexistent");
    assert_eq!(s.function, "unknown");
}

#[test]
fn status_schema_has_no_inputs() {
    assert!(schemas("status").inputs.is_empty());
}

#[test]
fn trigger_schema_has_no_inputs() {
    assert!(schemas("trigger").inputs.is_empty());
}

#[test]
fn tasks_add_requires_title() {
    let s = schemas("tasks_add");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"title"));
}

#[test]
fn tasks_update_requires_task_id() {
    let s = schemas("tasks_update");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"task_id"));
}

#[test]
fn tasks_remove_requires_task_id() {
    let s = schemas("tasks_remove");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"task_id"));
}

#[test]
fn escalations_approve_requires_escalation_id() {
    let s = schemas("escalations_approve");
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "escalation_id" && f.required));
}

#[test]
fn escalations_dismiss_requires_escalation_id() {
    let s = schemas("escalations_dismiss");
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "escalation_id" && f.required));
}

#[test]
fn log_list_has_optional_inputs() {
    let s = schemas("log_list");
    for input in &s.inputs {
        assert!(
            !input.required,
            "log_list input '{}' should be optional",
            input.name
        );
    }
}

#[test]
fn tasks_list_has_optional_enabled_only() {
    let s = schemas("tasks_list");
    let enabled = s.inputs.iter().find(|f| f.name == "enabled_only");
    assert!(enabled.is_some_and(|f| !f.required));
}

// ── Field helpers ──────────────────────────────────────────────

#[test]
fn field_helper_is_required() {
    let f = field("name", TypeSchema::String, "desc");
    assert!(f.required);
}

#[test]
fn field_req_helper_is_required() {
    let f = field_req("name", TypeSchema::String, "desc");
    assert!(f.required);
}

#[test]
fn field_opt_helper_is_not_required() {
    let f = field_opt("name", TypeSchema::String, "desc");
    assert!(!f.required);
}
