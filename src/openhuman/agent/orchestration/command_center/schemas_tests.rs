use super::*;

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert_eq!(schemas.len(), 2);
    assert!(schemas.iter().all(|s| s.namespace == "agent_work"));
    assert_eq!(schema_for("agent_work_list").function, "list");
    assert_eq!(schema_for("agent_work_control").function, "control");
}
