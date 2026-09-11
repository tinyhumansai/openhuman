use super::*;

#[test]
fn schema_namespace_and_function_are_stable() {
    let s = dashboard_schemas("dashboard_model_health");
    assert_eq!(s.namespace, "dashboard");
    assert_eq!(s.function, "model_health");
}

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_dashboard_controller_schemas().len(),
        all_dashboard_registered_controllers().len()
    );
}
