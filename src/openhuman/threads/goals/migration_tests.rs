use super::*;
use tinyagents_graph::goals::{store, ThreadGoalStatus};

#[tokio::test]
async fn migrates_legacy_goal_into_tinyagents_store() {
    let temp = tempfile::tempdir().unwrap();
    let legacy_dir = temp.path().join(LEGACY_GOALS_DIR);
    tokio::fs::create_dir_all(&legacy_dir).await.unwrap();
    let goal = ThreadGoal {
        thread_id: "thread-1".into(),
        goal_id: "legacy-id".into(),
        objective: "legacy objective".into(),
        status: ThreadGoalStatus::Active,
        token_budget: Some(100),
        tokens_used: 10,
        time_used_seconds: 2,
        created_at_ms: 1,
        updated_at_ms: 2,
        continuation_suppressed: false,
    };
    let path = legacy_goal_path(temp.path(), &goal.thread_id).unwrap();
    tokio::fs::write(&path, serde_json::to_vec(&goal).unwrap())
        .await
        .unwrap();

    let report = migrate_legacy_goals(temp.path()).await.unwrap();
    assert_eq!(
        report,
        GoalMigrationReport {
            total: 1,
            copied: 1,
            skipped: 0
        }
    );
    assert_eq!(
        store::get(&goals_store(temp.path()), "thread-1")
            .await
            .unwrap()
            .unwrap(),
        goal
    );
    assert!(!path.exists());
}
