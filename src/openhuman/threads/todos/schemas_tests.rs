use super::*;

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_controller_schemas().len(),
        all_registered_controllers().len()
    );
}

#[test]
fn schemas_have_todos_namespace() {
    for schema in all_controller_schemas() {
        assert_eq!(schema.namespace, "todos", "function={}", schema.function);
    }
}
