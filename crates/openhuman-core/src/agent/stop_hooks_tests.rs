use super::*;
use std::sync::Arc;

#[test]
fn host_tool_cap_only_narrows_the_default_and_keeps_zero() {
    assert_eq!(tool_call_limit(10, None), 80);
    assert_eq!(tool_call_limit(10, Some(4)), 4);
    // A cap wider than the product default never widens it.
    assert_eq!(tool_call_limit(10, Some(1_000)), 80);
    assert_eq!(tool_call_limit(1, Some(100)), 8);
    // Zero is a real cap: no tool may run.
    assert_eq!(tool_call_limit(10, Some(0)), 0);
}

#[test]
fn host_tool_cap_and_stop_hooks_flow_into_the_run_context_and_its_children() {
    let hook: Arc<dyn StopHook<tinyagents_harness::cost::TurnCost>> =
        Arc::new(BudgetStopHook::new(1.0));
    let mut hooks = crate::agent::hooks::HookScope::default();
    hooks.push_stop(Arc::clone(&hook));
    let env = crate::agent::turn_env::HostTurnEnv {
        tool_call_limit: Some(3),
        hooks,
        ..Default::default()
    };
    let context = crate::agent::tinyagents::host::OpenHumanRunContext::new().with_host_env(&env);
    assert_eq!(context.tool_call_limit, Some(3));
    assert_eq!(context.stop_hooks.len(), 1);
    assert_eq!(context.stop_hooks[0].name(), "budget");
    // Sub-agents inherit both through the explicit child carrier.
    let child = context.child();
    assert_eq!(child.tool_call_limit, Some(3));
    assert_eq!(child.stop_hooks.len(), 1);
    // A context built without an env carries nothing ambient.
    let bare = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    assert_eq!(bare.tool_call_limit, None);
    assert!(bare.stop_hooks.is_empty());
}

#[tokio::test]
async fn budget_stop_hook_fails_closed_and_stops_at_cap() {
    let hook = BudgetStopHook::new(1.0);
    let cost = TurnCost {
        cost: tinyagents_harness::cost::CostTally {
            known_usd: 1.0,
            source: tinyagents_harness::cost::CostSource::Charged,
        },
        ..TurnCost::default()
    };
    let state = TurnState {
        iteration: 1,
        max_iterations: 10,
        cost: &cost,
        model: "test",
    };
    assert!(matches!(
        hook.check(&state).await,
        StopDecision::Stop { .. }
    ));
    assert!(matches!(
        BudgetStopHook::new(f64::NAN).check(&state).await,
        StopDecision::Stop { .. }
    ));
}
