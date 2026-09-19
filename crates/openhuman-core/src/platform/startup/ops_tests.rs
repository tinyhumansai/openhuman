use super::run_workspace_migrations;
use std::fs;
use tempfile::TempDir;

#[test]
fn workspace_startup_migrates_transcripts_once() {
    let tempdir = TempDir::new().unwrap();
    let workspace = tempdir.path();
    let legacy = workspace
        .join("session_raw")
        .join("01052026")
        .join("1714000000_main.jsonl");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "legacy transcript").unwrap();

    run_workspace_migrations(workspace);

    let canonical = workspace.join("session_raw/1714000000_main.jsonl");
    let marker = workspace.join("state/migrations/session_layout_v1.done");
    assert_eq!(fs::read_to_string(&canonical).unwrap(), "legacy transcript");
    let first_marker = fs::read_to_string(&marker).unwrap();

    // A newly-arrived legacy artifact proves startup observes the marker rather
    // than re-running the migration on every process launch.
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "must remain legacy after marker").unwrap();
    run_workspace_migrations(workspace);

    assert_eq!(fs::read_to_string(&canonical).unwrap(), "legacy transcript");
    assert_eq!(
        fs::read_to_string(&legacy).unwrap(),
        "must remain legacy after marker"
    );
    assert_eq!(fs::read_to_string(marker).unwrap(), first_marker);
}
