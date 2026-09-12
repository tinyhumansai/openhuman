use super::*;

#[tokio::test]
async fn legacy_migration_copies_once_without_replacing_tinyagents_data() {
    let workspace = tempfile::tempdir().expect("workspace");
    let legacy_dir = workspace.path().join("agent_task_boards");
    tokio::fs::create_dir_all(&legacy_dir)
        .await
        .expect("legacy dir");
    let mut legacy = TaskBoard::empty("thread-1");
    legacy
        .cards
        .push(tinyagents_graph::todos::TaskBoardCard::new("legacy"));
    tokio::fs::write(
        legacy_dir.join("thread-1.json"),
        serde_json::to_vec(&legacy).expect("encode legacy"),
    )
    .await
    .expect("write legacy");

    let first = migrate_legacy_task_boards(workspace.path())
        .await
        .expect("first migration");
    assert_eq!(
        first,
        TaskBoardMigrationReport {
            total: 1,
            copied: 1,
            skipped: 0,
        }
    );

    let store = todos_store(workspace.path());
    todos::clear(&store, "thread-1").await.expect("edit board");
    let second = migrate_legacy_task_boards(workspace.path())
        .await
        .expect("second migration");
    assert_eq!(second.copied, 0);
    assert_eq!(second.skipped, 1);
    assert!(
        todos::get(&store, "thread-1")
            .await
            .expect("get board")
            .expect("present")
            .cards
            .is_empty(),
        "existing TinyAgents board must remain authoritative"
    );
}
