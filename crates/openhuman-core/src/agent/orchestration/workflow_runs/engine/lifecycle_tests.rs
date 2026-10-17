use super::*;
use crate::agent::session_store::ledger_sync::upsert_workflow_run;
use serde_json::json;

#[tokio::test]
async fn stop_fences_the_driver_and_makes_a_running_phase_resumable() {
    let workspace = tempfile::tempdir().expect("workspace");
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    let run = upsert_workflow_run(
        &config.workspace_dir,
        WorkflowRunUpsert {
            id: "stop-resume-lifecycle".into(),
            definition_id: "parallel_research_cross_check".into(),
            parent_thread_id: None,
            input: json!({"question":"q"}),
            phase_states: json!({
                "decompose": {"status":"completed", "outputs": []},
                "research": {"status":"running", "outputs": []}
            }),
            child_run_ids: vec!["live-child".into()],
            status: WorkflowRunStatus::Running,
            summary: None,
            started_at: None,
            completed_at: None,
        },
    )
    .expect("seed run");

    let stopped = stop_workflow_run(&config, &run.id)
        .await
        .expect("stop succeeds")
        .expect("row exists");
    assert_eq!(stopped.status, WorkflowRunStatus::Interrupted);
    assert_eq!(stopped.phase_states["research"]["status"], json!("pending"));
    assert!(stopped.lease_owner.is_none());
    assert!(
        stopped.revision > run.revision,
        "stop must fence the old driver"
    );
}
