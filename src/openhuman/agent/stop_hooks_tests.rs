use super::*;
use crate::openhuman::inference::provider::UsageInfo;

fn cost_with_usd(usd: f64) -> TurnCost {
    let mut tc = TurnCost::new();
    tc.add_call(
        "agentic-v1",
        &UsageInfo {
            charged_amount_usd: usd,
            ..Default::default()
        },
    );
    tc
}

#[tokio::test]
async fn budget_hook_continues_under_cap() {
    let cost = cost_with_usd(0.10);
    let hook = BudgetStopHook::new(1.00);
    let ctx = TurnState {
        iteration: 1,
        max_iterations: 10,
        cost: &cost,
        model: "agentic-v1",
    };
    assert!(matches!(hook.check(&ctx).await, StopDecision::Continue));
}

#[tokio::test]
async fn budget_hook_stops_at_cap() {
    let cost = cost_with_usd(1.50);
    let hook = BudgetStopHook::new(1.00);
    let ctx = TurnState {
        iteration: 2,
        max_iterations: 10,
        cost: &cost,
        model: "agentic-v1",
    };
    match hook.check(&ctx).await {
        StopDecision::Stop { reason } => {
            assert!(reason.contains("$1.5000"));
            assert!(reason.contains("$1.0000"));
        }
        other => panic!("expected Stop, got {other:?}"),
    }
}

#[tokio::test]
async fn budget_hook_fails_closed_on_nan_cap() {
    // NaN comparisons always return false, so without the guard
    // `spent >= NaN` would silently disable the cap forever.
    let cost = cost_with_usd(1.0);
    let hook = BudgetStopHook::new(f64::NAN);
    let ctx = TurnState {
        iteration: 1,
        max_iterations: 10,
        cost: &cost,
        model: "agentic-v1",
    };
    match hook.check(&ctx).await {
        StopDecision::Stop { reason } => assert!(reason.contains("invalid budget cap")),
        other => panic!("expected Stop on NaN cap, got {other:?}"),
    }
}

#[tokio::test]
async fn budget_hook_fails_closed_on_non_positive_cap() {
    let cost = TurnCost::new();
    let ctx = TurnState {
        iteration: 1,
        max_iterations: 10,
        cost: &cost,
        model: "agentic-v1",
    };
    for bad in [0.0, -1.0, f64::NEG_INFINITY, f64::INFINITY] {
        let hook = BudgetStopHook::new(bad);
        assert!(
            matches!(hook.check(&ctx).await, StopDecision::Stop { .. }),
            "cap {bad} should stop"
        );
    }
}

#[tokio::test]
async fn max_iterations_hook_stops_when_exceeded() {
    let cost = TurnCost::new();
    let hook = MaxIterationsStopHook::new(3);
    let ctx = TurnState {
        iteration: 4,
        max_iterations: 10,
        cost: &cost,
        model: "agentic-v1",
    };
    assert!(matches!(hook.check(&ctx).await, StopDecision::Stop { .. }));
}

#[tokio::test]
async fn current_stop_hooks_returns_empty_outside_scope() {
    assert!(current_stop_hooks().is_empty());
}

#[tokio::test]
async fn with_stop_hooks_installs_visible_within_scope() {
    let hooks: Vec<Arc<dyn StopHook>> = vec![Arc::new(BudgetStopHook::new(0.5))];
    with_stop_hooks(hooks, async {
        let visible = current_stop_hooks();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name(), "budget");
    })
    .await;
    assert!(current_stop_hooks().is_empty());
}
