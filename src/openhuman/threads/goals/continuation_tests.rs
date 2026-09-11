use super::*;

fn config_with(enabled: bool, idle_minutes: u32, workspace: &Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.heartbeat.goal_continuation_enabled = enabled;
    config.heartbeat.goal_idle_minutes = idle_minutes;
    config
}

#[tokio::test]
async fn tick_is_noop_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    // Active, long-idle goal present — but the feature is off.
    store::set(tmp.path(), "t", "obj", None).await.unwrap();
    let config = config_with(false, 1, tmp.path());
    // Must not panic / must return promptly; nothing to assert beyond no-op.
    run_continuation_tick(&config).await;
    // Goal untouched (not suppressed) since the feature is disabled.
    let g = store::get(tmp.path(), "t").await.unwrap().unwrap();
    assert!(!g.continuation_suppressed);
}

#[tokio::test]
async fn candidate_filter_respects_status_idle_and_suppression() {
    // This exercises the selection predicate without dispatching a turn by
    // keeping the feature enabled but pointing at a fresh (non-idle) goal.
    let tmp = tempfile::tempdir().unwrap();
    // Fresh goal: updated_at is "now", so with a 60-min idle window it is
    // NOT a candidate and the tick stays a no-op (no agent build attempted).
    store::set(tmp.path(), "fresh", "obj", None).await.unwrap();
    let config = config_with(true, 60, tmp.path());
    run_continuation_tick(&config).await;
    let g = store::get(tmp.path(), "fresh").await.unwrap().unwrap();
    assert!(
        !g.continuation_suppressed,
        "fresh goal must not be continued/suppressed"
    );
}

#[test]
fn continuation_prompt_names_objective_and_guards() {
    let p = continuation_prompt("ship the release");
    assert!(p.contains("ship the release"));
    assert!(p.contains("goal_complete"));
    assert!(p.contains("not auto-approved"));
}
