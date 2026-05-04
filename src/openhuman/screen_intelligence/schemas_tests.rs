use super::*;

#[test]
fn catalog_counts_match_and_nonempty() {
    let s = all_controller_schemas();
    let h = all_registered_controllers();
    assert_eq!(s.len(), h.len());
    assert!(s.len() >= 10);
}

#[test]
fn all_schemas_use_accessibility_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(
            s.namespace, "screen_intelligence",
            "function {}",
            s.function
        );
        assert!(!s.description.is_empty());
        assert!(!s.outputs.is_empty());
    }
}

#[test]
fn unknown_function_returns_unknown_schema() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
}

#[test]
fn every_known_key_resolves_to_non_unknown() {
    let keys = [
        "status",
        "request_permissions",
        "request_permission",
        "refresh_permissions",
        "start_session",
        "stop_session",
        "capture_now",
        "capture_image_ref",
        "input_action",
        "vision_recent",
        "vision_flush",
        "capture_test",
        "globe_listener_start",
        "globe_listener_poll",
        "globe_listener_stop",
    ];
    for k in keys {
        let s = schemas(k);
        assert_eq!(s.namespace, "screen_intelligence");
        assert_ne!(s.function, "unknown", "key `{k}` fell through");
    }
}

#[test]
fn registered_controllers_use_accessibility_namespace() {
    for h in all_registered_controllers() {
        assert_eq!(h.schema.namespace, "screen_intelligence");
        assert!(!h.schema.function.is_empty());
    }
}

#[test]
fn status_schema_has_no_inputs() {
    let s = schemas("status");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
}

#[test]
fn request_permissions_schema_has_no_inputs() {
    assert!(schemas("request_permissions").inputs.is_empty());
}

#[test]
fn request_permission_requires_permission_input() {
    let s = schemas("request_permission");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "permission");
    assert!(s.inputs[0].required);
}

#[test]
fn refresh_permissions_schema_has_no_inputs() {
    assert!(schemas("refresh_permissions").inputs.is_empty());
}

#[test]
fn start_session_schema_requires_consent() {
    let s = schemas("start_session");
    let consent = s.inputs.iter().find(|f| f.name == "consent").unwrap();
    assert!(consent.required);
}

#[test]
fn stop_session_schema_has_optional_reason() {
    let s = schemas("stop_session");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "reason");
    assert!(!s.inputs[0].required);
}

#[test]
fn capture_now_schema_has_optional_inputs() {
    let s = schemas("capture_now");
    for input in &s.inputs {
        assert!(
            !input.required,
            "capture_now input '{}' should be optional",
            input.name
        );
    }
}

#[test]
fn capture_image_ref_schema_has_no_inputs() {
    let s = schemas("capture_image_ref");
    assert!(s.inputs.is_empty());
}

#[test]
fn input_action_schema_requires_action() {
    let s = schemas("input_action");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"action"));
}

#[test]
fn vision_recent_schema() {
    let s = schemas("vision_recent");
    assert!(!s.description.is_empty());
}

#[test]
fn vision_flush_schema_has_no_inputs() {
    assert!(schemas("vision_flush").inputs.is_empty());
}

#[test]
fn capture_test_schema() {
    let s = schemas("capture_test");
    assert_eq!(s.function, "capture_test");
}

#[test]
fn globe_listener_start_schema() {
    let s = schemas("globe_listener_start");
    assert_eq!(s.function, "globe_listener_start");
}

#[test]
fn globe_listener_poll_schema() {
    let s = schemas("globe_listener_poll");
    assert_eq!(s.function, "globe_listener_poll");
}

#[test]
fn globe_listener_stop_schema() {
    let s = schemas("globe_listener_stop");
    assert_eq!(s.function, "globe_listener_stop");
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}
