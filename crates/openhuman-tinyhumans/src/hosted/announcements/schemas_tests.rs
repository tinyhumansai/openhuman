use super::*;

#[test]
fn get_latest_schema_has_expected_namespace_and_no_inputs() {
    let schema = announcements_schemas("announcements_get_latest");
    assert_eq!(schema.namespace, "announcements");
    assert_eq!(schema.function, "get_latest");
    assert!(schema.inputs.is_empty());
    assert_eq!(schema.outputs.len(), 1);
    assert_eq!(schema.outputs[0].name, "announcement");
}

#[test]
fn unknown_function_falls_back() {
    let schema = announcements_schemas("nope");
    assert_eq!(schema.function, "unknown");
}

#[test]
fn registers_exactly_one_controller() {
    assert_eq!(all_announcements_registered_controllers().len(), 1);
    assert_eq!(all_announcements_controller_schemas().len(), 1);
}
