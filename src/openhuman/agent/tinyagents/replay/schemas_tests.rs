use super::*;

#[test]
fn controller_inventory_is_stable() {
    let schemas = all_agent_replay_controller_schemas();
    assert_eq!(schemas.len(), 3);
    assert!(schemas.iter().all(|s| s.namespace == "agent"));
    let functions: Vec<&str> = schemas.iter().map(|s| s.function).collect();
    assert!(functions.contains(&"run_events"));
    assert!(functions.contains(&"run_status"));
    assert!(functions.contains(&"runs_active"));

    let controllers = all_agent_replay_registered_controllers();
    assert_eq!(controllers.len(), 3);
    // rpc method names follow openhuman.<namespace>_<function>.
    let methods: Vec<String> = controllers.iter().map(|c| c.rpc_method_name()).collect();
    assert!(methods.contains(&"openhuman.agent_run_events".to_string()));
    assert!(methods.contains(&"openhuman.agent_run_status".to_string()));
    assert!(methods.contains(&"openhuman.agent_runs_active".to_string()));
}

#[tokio::test]
async fn run_events_rejects_missing_run_id() {
    let err = handle_run_events(Map::new()).await.unwrap_err();
    assert!(err.contains("invalid params"), "{err}");
}
