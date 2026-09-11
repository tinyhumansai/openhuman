use super::*;

#[test]
fn all_schemas_returns_nine() {
    assert_eq!(all_controller_schemas().len(), 9);
}

#[test]
fn all_controllers_returns_nine() {
    assert_eq!(all_registered_controllers().len(), 9);
}

#[test]
fn all_schemas_use_service_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "service");
        assert!(!s.description.is_empty());
    }
}

#[test]
fn lifecycle_schemas_have_no_inputs_except_self_signals() {
    for fn_name in ["install", "start", "stop", "status", "uninstall"] {
        let s = schemas(fn_name);
        assert!(
            s.inputs.is_empty(),
            "{fn_name} should have no inputs, got {}",
            s.inputs.len()
        );
    }
}

#[test]
fn restart_and_shutdown_schemas_have_optional_source_and_reason() {
    for fn_name in ["restart", "shutdown"] {
        let s = schemas(fn_name);
        assert_eq!(s.function, fn_name);
        assert_eq!(s.inputs.len(), 2, "{fn_name} should have 2 inputs");
        for input in &s.inputs {
            assert!(
                !input.required,
                "{fn_name} input '{}' should be optional",
                input.name
            );
        }
    }
}

#[test]
fn daemon_host_get_schema_has_no_inputs() {
    let s = schemas("daemon_host_get");
    assert!(s.inputs.is_empty());
}

#[test]
fn daemon_host_set_requires_show_tray() {
    let s = schemas("daemon_host_set");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "show_tray");
    assert!(s.inputs[0].required);
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("nonexistent");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "service");
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    assert_eq!(s.len(), c.len());
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}

#[test]
fn known_functions_resolve_correctly() {
    for fn_name in [
        "install",
        "restart",
        "shutdown",
        "start",
        "stop",
        "status",
        "uninstall",
        "daemon_host_get",
        "daemon_host_set",
    ] {
        let s = schemas(fn_name);
        assert_ne!(s.function, "unknown", "{fn_name} fell through");
    }
}
