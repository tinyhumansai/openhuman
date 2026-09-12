use super::*;

#[test]
fn schemas_and_controllers_stay_in_lockstep_with_list_queue_present() {
    // Parity + known-op presence instead of a magic `== 2` count, which
    // would break on any legitimate third controller (plan.md §3).
    crate::core::all::assert_schema_controller_parity(
        &all_provider_surfaces_controller_schemas(),
        &all_provider_surfaces_registered_controllers(),
        "list_queue",
    );
}

#[test]
fn list_queue_schema_has_no_inputs() {
    let schema = schemas("list_queue");
    assert!(schema.inputs.is_empty());
    assert_eq!(schema.namespace, "provider_surfaces");
}
