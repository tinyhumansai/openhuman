use super::*;

#[test]
fn rollback_refuses_everything_except_a_ready_deployment() {
    for status in ["queued", "building", "failed", "canceled", "unknown"] {
        let deployment = json!({"status": status});
        assert!(!rollback_target_is_ready(&deployment), "{status}");
        assert_ne!(rollback_status(&deployment), "Ready");
    }
    let ready = json!({"status": "ready"});
    assert!(rollback_target_is_ready(&ready));
    assert_eq!(rollback_status(&ready), "Ready");
}

#[test]
fn launch_summary_keeps_the_model_facing_status_and_wait_guidance() {
    let launch = json!({
        "site": {"name": "demo"},
        "created_site": true,
        "deployment": {"id": "dpl_1", "status": "building", "url": "https://demo.example"},
        "database": null,
        "domains": []
    });
    let summary = launch_summary(&launch);
    assert!(summary.contains("Site **demo** (created), deployment `dpl_1` is Building."));
    assert!(summary.contains("poll `hosting_deployment_status`"));
}
