use super::*;
use crate::openhuman::agent::cost::TurnCost;
use crate::openhuman::inference::provider::UsageInfo;

fn cost_with_tokens(input: u64, output: u64) -> TurnCost {
    let mut tc = TurnCost::new();
    tc.add_call(
        "agentic-v1",
        &UsageInfo {
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        },
    );
    tc
}

#[tokio::test]
async fn account_turn_charges_active_goal_and_trips_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-acct", async {
        store::set(&dir, "t-acct", "obj", Some(100)).await.unwrap();
        account_turn_against_goal(&dir, 80, 40, 3).await; // 120 >= 100
        let g = store::get(&dir, "t-acct").await.unwrap().unwrap();
        assert_eq!(g.tokens_used, 120);
        assert_eq!(g.status, ThreadGoalStatus::BudgetLimited);
    })
    .await;
}

#[tokio::test]
async fn account_turn_skips_non_active_goal() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-paused", async {
        store::set(&dir, "t-paused", "obj", Some(1000))
            .await
            .unwrap();
        store::pause(&dir, "t-paused").await.unwrap();
        account_turn_against_goal(&dir, 500, 500, 1).await;
        let g = store::get(&dir, "t-paused").await.unwrap().unwrap();
        assert_eq!(g.tokens_used, 0, "paused goal must not accrue usage");
    })
    .await;
}

#[tokio::test]
async fn complete_for_current_thread_settles_active_goal() {
    // A goal an originating task left behind is settled to `Complete` so a
    // later turn stops re-injecting its `[active_goal]` block (#1725).
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-complete", async {
        store::set(&dir, "t-complete", "obj", Some(1000))
            .await
            .unwrap();
        complete_for_current_thread(&dir).await;
        let g = store::get(&dir, "t-complete").await.unwrap().unwrap();
        assert_eq!(
            g.status,
            ThreadGoalStatus::Complete,
            "an originating task's settle must mark the goal Complete, not leave it Active"
        );
        assert!(
            !g.status.is_active(),
            "a completed goal must not stay active (it would keep steering unrelated turns)"
        );
    })
    .await;
}

#[tokio::test]
async fn clear_for_current_thread_removes_the_goal_row() {
    // Clearing deletes the row entirely, so a later turn loads `None` and
    // injects no goal block at all — the strongest anti-leak guarantee.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-clear", async {
        store::set(&dir, "t-clear", "obj", Some(1000))
            .await
            .unwrap();
        clear_for_current_thread(&dir).await;
        let g = store::get(&dir, "t-clear").await.unwrap();
        assert!(
            g.is_none(),
            "clearing must delete the goal row so no [active_goal] can leak forward"
        );
    })
    .await;
}

#[tokio::test]
async fn complete_and_clear_are_safe_without_a_goal_or_thread() {
    // Best-effort contract: both are no-ops (never panic) when there is no
    // goal for the thread, and when called outside any thread scope.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-empty", async {
        complete_for_current_thread(&dir).await;
        clear_for_current_thread(&dir).await;
        assert!(store::get(&dir, "t-empty").await.unwrap().is_none());
    })
    .await;
    // No ambient thread scope at all.
    complete_for_current_thread(&dir).await;
    clear_for_current_thread(&dir).await;
}

#[tokio::test]
async fn account_turn_clears_suppression_without_losing_usage() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-suppressed", async {
        let goal = store::set(&dir, "t-suppressed", "obj", Some(1000))
            .await
            .unwrap();
        store::set_continuation_suppressed_if(&dir, "t-suppressed", &goal.goal_id, true)
            .await
            .unwrap();

        account_turn_against_goal(&dir, 80, 40, 3).await;

        let updated = store::get(&dir, "t-suppressed").await.unwrap().unwrap();
        assert_eq!(updated.goal_id, goal.goal_id);
        assert!(!updated.continuation_suppressed);
        assert_eq!(updated.tokens_used, 120);
        assert_eq!(updated.time_used_seconds, 3);
    })
    .await;
}

#[tokio::test]
async fn budget_stop_hook_fires_on_crossing() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let goal = store::set(&dir, "t-hook", "obj", Some(1000)).await.unwrap();
    // 600 already used in a prior turn.
    store::account_usage(&dir, "t-hook", &goal.goal_id, 600, 0)
        .await
        .unwrap();
    let goal = store::get(&dir, "t-hook").await.unwrap().unwrap();
    let hook = GoalBudgetStopHook::for_goal(&dir, &goal).expect("budgeted active goal");

    // This turn so far: 300 in + 200 out = 500. 600 + 500 = 1100 >= 1000.
    let cost = cost_with_tokens(300, 200);
    let ctx = TurnState {
        iteration: 2,
        max_iterations: 10,
        cost: &cost,
        model: "agentic-v1",
    };
    assert!(matches!(hook.check(&ctx).await, StopDecision::Stop { .. }));

    // Under the cap continues.
    let small = cost_with_tokens(100, 100); // 600 + 200 = 800 < 1000
    let ctx2 = TurnState {
        iteration: 1,
        max_iterations: 10,
        cost: &small,
        model: "agentic-v1",
    };
    assert!(matches!(hook.check(&ctx2).await, StopDecision::Continue));
}

#[tokio::test]
async fn no_hook_without_budget_or_when_inactive() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let no_budget = store::set(&dir, "a", "obj", None).await.unwrap();
    assert!(GoalBudgetStopHook::for_goal(&dir, &no_budget).is_none());
    store::set(&dir, "b", "obj", Some(100)).await.unwrap();
    store::pause(&dir, "b").await.unwrap();
    let paused = store::get(&dir, "b").await.unwrap().unwrap();
    assert!(GoalBudgetStopHook::for_goal(&dir, &paused).is_none());
}
