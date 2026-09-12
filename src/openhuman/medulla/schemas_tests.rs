use super::*;

#[test]
fn every_declared_schema_has_a_registered_controller() {
    let schemas = all_medulla_controller_schemas();
    let controllers = all_medulla_registered_controllers();
    assert_eq!(
        schemas.len(),
        controllers.len(),
        "a declared schema without a handler is unreachable, and vice versa"
    );
    for (schema, controller) in schemas.iter().zip(controllers.iter()) {
        assert_eq!(schema.namespace, controller.schema.namespace);
        assert_eq!(schema.function, controller.schema.function);
    }
}

#[test]
fn all_schemas_share_the_medulla_namespace() {
    for schema in all_medulla_controller_schemas() {
        assert_eq!(schema.namespace, "medulla");
        assert!(!schema.description.is_empty());
    }
}

#[test]
fn rpc_method_names_follow_the_crate_convention() {
    let names: Vec<String> = all_medulla_registered_controllers()
        .iter()
        .map(|c| c.rpc_method_name())
        .collect();
    assert_eq!(
        names,
        vec![
            "openhuman.medulla_status",
            "openhuman.medulla_list_sessions",
            "openhuman.medulla_create_session",
            "openhuman.medulla_get_session",
            "openhuman.medulla_send_message",
            "openhuman.medulla_abort",
            "openhuman.medulla_list_messages",
            "openhuman.medulla_list_events",
            "openhuman.medulla_roster",
        ]
    );
}
