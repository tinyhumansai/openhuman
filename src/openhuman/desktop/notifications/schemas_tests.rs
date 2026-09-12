use super::*;

#[test]
fn all_controller_schemas_covers_registered_functions() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(
        names,
        vec![
            "ingest",
            "list",
            "mark_read",
            "settings_get",
            "settings_set",
            "dismiss",
            "mark_acted",
            "stats",
            "core_list",
            "core_mark_read",
        ]
    );
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 10);
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(
        names,
        vec![
            "ingest",
            "list",
            "mark_read",
            "settings_get",
            "settings_set",
            "dismiss",
            "mark_acted",
            "stats",
            "core_list",
            "core_mark_read",
        ]
    );
}

#[test]
fn schemas_dismiss_and_mark_acted_require_id_and_return_ok() {
    let dismiss = schemas("dismiss");
    assert_eq!(dismiss.inputs.len(), 1);
    assert_eq!(dismiss.inputs[0].name, "id");
    assert_eq!(dismiss.inputs[0].ty, TypeSchema::String);
    assert!(dismiss.inputs[0].required);
    assert_eq!(dismiss.outputs.len(), 1);
    assert_eq!(dismiss.outputs[0].name, "ok");
    assert_eq!(dismiss.outputs[0].ty, TypeSchema::Bool);
    assert!(dismiss.outputs[0].required);

    let mark_acted = schemas("mark_acted");
    assert_eq!(mark_acted.inputs.len(), 1);
    assert_eq!(mark_acted.inputs[0].name, "id");
    assert_eq!(mark_acted.inputs[0].ty, TypeSchema::String);
    assert!(mark_acted.inputs[0].required);
    assert_eq!(mark_acted.outputs.len(), 1);
    assert_eq!(mark_acted.outputs[0].name, "ok");
    assert_eq!(mark_acted.outputs[0].ty, TypeSchema::Bool);
    assert!(mark_acted.outputs[0].required);
}

#[test]
fn schemas_stats_matches_notification_stats_shape() {
    let stats = schemas("stats");
    assert!(stats.inputs.is_empty());
    assert_eq!(stats.outputs.len(), 5);

    let expected = [
        ("total", TypeSchema::I64),
        ("unread", TypeSchema::I64),
        ("unscored", TypeSchema::I64),
        ("by_provider", TypeSchema::Map(Box::new(TypeSchema::I64))),
        ("by_action", TypeSchema::Map(Box::new(TypeSchema::I64))),
    ];

    for (name, ty) in expected {
        let field = stats
            .outputs
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("missing stats output field `{name}`"));
        assert_eq!(field.ty, ty, "unexpected type for stats.{name}");
        assert!(field.required, "stats.{name} should be required");
    }
}

#[test]
fn schemas_ingest_requires_provider_title_body_raw_payload() {
    let s = schemas("ingest");
    assert_eq!(s.namespace, "notification");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"provider"));
    assert!(required.contains(&"title"));
    assert!(required.contains(&"body"));
    assert!(required.contains(&"raw_payload"));
}

#[test]
fn schemas_list_all_inputs_optional() {
    let s = schemas("list");
    assert!(s.inputs.iter().all(|f| !f.required));
}

#[test]
fn schemas_mark_read_requires_id() {
    let s = schemas("mark_read");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "id");
    assert!(s.inputs[0].required);
}

#[test]
fn schemas_and_registered_controllers_have_bidirectional_parity() {
    let schema_functions: std::collections::BTreeSet<_> = all_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect();
    let handler_functions: std::collections::BTreeSet<_> = all_registered_controllers()
        .into_iter()
        .map(|controller| controller.schema.function)
        .collect();

    assert_eq!(schema_functions, handler_functions);
}

#[test]
fn schemas_unknown_returns_placeholder() {
    let s = schemas("does-not-exist");
    assert_eq!(s.function, "unknown");
}
