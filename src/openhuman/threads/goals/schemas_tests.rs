use super::*;

#[test]
fn registers_all_controllers() {
    let controllers = all_thread_goals_registered_controllers();
    assert_eq!(controllers.len(), FUNCTIONS.len());
    let methods: Vec<String> = controllers
        .iter()
        .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
        .collect();
    for f in FUNCTIONS {
        let expected = format!("thread_goals.{f}");
        assert!(methods.contains(&expected), "missing {expected}");
    }
}

#[test]
fn schemas_and_controllers_stay_in_sync() {
    assert_eq!(
        all_thread_goals_controller_schemas().len(),
        all_thread_goals_registered_controllers().len()
    );
}
