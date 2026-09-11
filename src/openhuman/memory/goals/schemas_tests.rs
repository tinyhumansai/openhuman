use super::*;

#[test]
fn registers_all_five_controllers() {
    let controllers = all_memory_goals_registered_controllers();
    assert_eq!(controllers.len(), 5);
    let methods: Vec<String> = controllers
        .iter()
        .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
        .collect();
    for expected in [
        "memory_goals.list",
        "memory_goals.add",
        "memory_goals.edit",
        "memory_goals.delete",
        "memory_goals.reflect",
    ] {
        assert!(
            methods.contains(&expected.to_string()),
            "missing {expected}"
        );
    }
}

#[test]
fn schemas_and_controllers_stay_in_sync() {
    assert_eq!(
        all_memory_goals_controller_schemas().len(),
        all_memory_goals_registered_controllers().len()
    );
}
