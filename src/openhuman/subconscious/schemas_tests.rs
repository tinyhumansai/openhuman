use super::*;

#[test]
fn all_schemas_returns_two() {
    assert_eq!(all_controller_schemas().len(), 2);
}

#[test]
fn all_controllers_returns_two() {
    assert_eq!(all_registered_controllers().len(), 2);
}

#[test]
fn status_and_trigger_are_registered() {
    let names: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.function)
        .collect();
    assert!(names.contains(&"status"));
    assert!(names.contains(&"trigger"));
}

#[test]
fn reflection_rpcs_are_removed() {
    let names: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.function)
        .collect();
    assert!(!names.contains(&"reflections_list"));
    assert!(!names.contains(&"reflections_act"));
    assert!(!names.contains(&"reflections_dismiss"));
}
