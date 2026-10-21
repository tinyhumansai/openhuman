use super::*;
#[tokio::test]
async fn account_turn_charges_active_goal_and_trips_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    tinyagents_harness_tools::goals::store::set(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-acct",
        "obj",
        Some(100),
    )
    .await
    .unwrap();
    account_turn_against_goal(&dir, Some("t-acct"), 80, 40, 3).await; // 120 >= 100
    let g = tinyagents_harness_tools::goals::store::get(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-acct",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(g.tokens_used, 120);
    assert_eq!(g.status, ThreadGoalStatus::BudgetLimited);
}
#[tokio::test]
async fn account_turn_skips_non_active_goal() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    tinyagents_harness_tools::goals::store::set(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-paused",
        "obj",
        Some(1000),
    )
    .await
    .unwrap();
    tinyagents_harness_tools::goals::store::pause(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-paused",
    )
    .await
    .unwrap();
    account_turn_against_goal(&dir, Some("t-paused"), 500, 500, 1).await;
    let g = tinyagents_harness_tools::goals::store::get(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-paused",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(g.tokens_used, 0, "paused goal must not accrue usage");
}

#[tokio::test]
async fn complete_for_thread_settles_active_goal() {
    // A goal an originating task left behind is settled to `Complete` so a
    // later turn stops re-injecting its `[active_goal]` block (#1725).
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    tinyagents_harness_tools::goals::store::set(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-complete",
        "obj",
        Some(1000),
    )
    .await
    .unwrap();
    complete_for_thread(&dir, Some("t-complete")).await;
    let g = tinyagents_harness_tools::goals::store::get(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-complete",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        g.status,
        ThreadGoalStatus::Complete,
        "an originating task's settle must mark the goal Complete, not leave it Active"
    );
    assert!(
        !g.status.is_active(),
        "a completed goal must not stay active (it would keep steering unrelated turns)"
    );
}

#[tokio::test]
async fn clear_for_thread_removes_the_goal_row() {
    // Clearing deletes the row entirely, so a later turn loads `None` and
    // injects no goal block at all — the strongest anti-leak guarantee.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    tinyagents_harness_tools::goals::store::set(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-clear",
        "obj",
        Some(1000),
    )
    .await
    .unwrap();
    clear_for_thread(&dir, Some("t-clear")).await;
    let g = tinyagents_harness_tools::goals::store::get(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-clear",
    )
    .await
    .unwrap();
    assert!(
        g.is_none(),
        "clearing must delete the goal row so no [active_goal] can leak forward"
    );
}

#[tokio::test]
async fn complete_and_clear_are_safe_without_a_goal_or_thread() {
    // Best-effort contract: both are no-ops (never panic) when there is no
    // goal for the thread, and when called outside any thread scope.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    complete_for_thread(&dir, Some("t-empty")).await;
    clear_for_thread(&dir, Some("t-empty")).await;
    assert!(tinyagents_harness_tools::goals::store::get(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "t-empty"
    )
    .await
    .unwrap()
    .is_none());
    // No ambient thread scope at all.
    complete_for_thread(&dir, None).await;
    clear_for_thread(&dir, None).await;
}
