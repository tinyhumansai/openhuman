use super::run_workspace_migrations;
use std::fs;
use tempfile::TempDir;

#[test]
fn workspace_startup_runs_welcome_migration_once() {
    let tempdir = TempDir::new().unwrap();
    let workspace = tempdir.path();
    let legacy = workspace.join("session_raw/1715000000_welcome_thread-abc.jsonl");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(
        &legacy,
        "{\"_meta\":{\"agent\":\"welcome_thread-abc\",\"thread_id\":\"thread-abc\"}}\n{\"role\":\"user\",\"content\":\"hi\"}\n",
    )
    .unwrap();

    run_workspace_migrations(&crate::core::runtime::DispatchCtx::ambient(), workspace);

    let canonical = workspace.join("session_raw/1715000000_orchestrator_thread-abc.jsonl");
    let marker = workspace.join("state/migrations/welcome_to_orchestrator_v1.done");
    let rewritten = fs::read_to_string(&canonical).unwrap();
    assert!(rewritten.contains("\"agent\":\"orchestrator_thread-abc\""));
    let first_marker = fs::read_to_string(&marker).unwrap();

    // A late welcome transcript proves startup observes the marker rather than
    // re-running the welcome-agent migration on every process launch.
    let late_legacy = workspace.join("session_raw/1715000001_welcome_thread-late.jsonl");
    fs::write(
        &late_legacy,
        "{\"_meta\":{\"agent\":\"welcome_thread-late\",\"thread_id\":\"thread-late\"}}\n",
    )
    .unwrap();
    run_workspace_migrations(&crate::core::runtime::DispatchCtx::ambient(), workspace);

    assert_eq!(
        fs::read_to_string(&late_legacy).unwrap(),
        "{\"_meta\":{\"agent\":\"welcome_thread-late\",\"thread_id\":\"thread-late\"}}\n"
    );
    assert_eq!(fs::read_to_string(marker).unwrap(), first_marker);
}
