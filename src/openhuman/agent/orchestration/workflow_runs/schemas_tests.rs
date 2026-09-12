use super::*;

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert_eq!(
        schemas.len(),
        6,
        "expected 3 read + 3 execution controllers"
    );
    assert!(schemas.iter().all(|s| s.namespace == "workflow_run"));
    assert_eq!(schema_for("workflow_run_get").function, "get");
    assert_eq!(schema_for("workflow_run_start").function, "start");
    assert_eq!(schema_for("workflow_run_stop").function, "stop");
    assert_eq!(schema_for("workflow_run_resume").function, "resume");
}
