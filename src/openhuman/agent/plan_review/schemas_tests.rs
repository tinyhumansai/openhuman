use super::*;

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_controller_schemas().len(),
        all_registered_controllers().len()
    );
}

#[test]
fn schema_uses_plan_review_namespace() {
    assert_eq!(schemas("decide").namespace, "plan_review");
}
