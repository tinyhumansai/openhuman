use super::*;

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert_eq!(schemas.len(), 11);
    assert!(schemas.iter().all(|s| s.namespace == "agent_team"));
    assert_eq!(schema_for("agent_team_claim_task").function, "claim_task");
    assert_eq!(
        schema_for("agent_team_complete_task").function,
        "complete_task"
    );
    assert_eq!(
        schema_for("agent_team_shutdown_member").function,
        "shutdown_member"
    );
}
