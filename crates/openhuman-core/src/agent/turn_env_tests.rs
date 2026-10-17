use super::*;
use crate::agent::stop_hooks::BudgetStopHook;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use std::sync::Arc;
use std::time::Duration;

fn full_env() -> (HostTurnEnv, CancellationToken) {
    let token = CancellationToken::new();
    let mut hooks = HookScope::default();
    hooks.push_stop(Arc::new(BudgetStopHook::new(2.0)));
    let env = HostTurnEnv {
        spawn_depth_limit: Some(1),
        cancellation: Some(token.clone()),
        turn_deadline: Some(TurnDeadline::starting_now(Duration::from_secs(900))),
        tool_call_limit: Some(5),
        hooks,
        ..Default::default()
    };
    (env, token)
}

/// A bare run context carries nothing from the surrounding task: the env is
/// the only way a host value reaches a turn.
#[test]
fn a_run_context_without_an_env_carries_no_host_policy() {
    let context = OpenHumanRunContext::new();
    assert!(context.model_budget.is_none());
    assert!(context.max_spawn_depth.is_none());
    assert!(context.turn_deadline.is_none());
    assert!(context.tool_call_limit.is_none());
    assert!(context.stop_hooks.is_empty());
    assert!(context.tool_hooks.is_empty());
    assert!(!context.cancellation.is_cancelled());
}

#[test]
fn the_env_reaches_the_run_context() {
    let (env, _) = full_env();
    let context = OpenHumanRunContext::new().with_host_env(&env);
    assert_eq!(context.max_spawn_depth, Some(1));
    assert_eq!(context.tool_call_limit, Some(5));
    assert!(context.turn_deadline.is_some());
    assert_eq!(context.stop_hooks.len(), 1);
}

/// A sub-agent fork inherits the host env, and the caller's cancellation
/// cascades into it.
#[test]
fn sub_agent_forks_inherit_the_env_and_the_callers_cancellation() {
    let (env, token) = full_env();
    let context = OpenHumanRunContext::new().with_host_env(&env);
    let child = context.child();
    assert_eq!(child.max_spawn_depth, Some(1));
    assert_eq!(child.tool_call_limit, Some(5));
    assert_eq!(child.stop_hooks.len(), 1);
    assert!(child.turn_deadline.is_some());
    assert!(!child.cancellation.is_cancelled());
    token.cancel();
    assert!(context.cancellation.is_cancelled());
    assert!(child.cancellation.is_cancelled());
}

/// The env is a plain value: it survives await points and a `tokio::spawn`,
/// which an ambient task scope would silently lose.
#[tokio::test]
async fn the_env_survives_await_points_and_spawned_tasks() {
    let (env, token) = full_env();
    let context = OpenHumanRunContext::new().with_host_env(&env);
    tokio::task::yield_now().await;
    let seen = tokio::spawn(async move {
        tokio::task::yield_now().await;
        (
            context.max_spawn_depth,
            context.tool_call_limit,
            context.cancellation.is_cancelled(),
        )
    });
    token.cancel();
    assert_eq!(seen.await.unwrap(), (Some(1), Some(5), true));
}

/// A detached background child keeps the policy but is not cancelled by the
/// interactive turn's Stop.
#[test]
fn detached_children_keep_policy_but_not_the_turns_cancellation() {
    let (env, token) = full_env();
    let detached = OpenHumanRunContext::new()
        .with_host_env(&env)
        .detached_child();
    assert_eq!(detached.tool_call_limit, Some(5));
    token.cancel();
    assert!(!detached.cancellation.is_cancelled());
}
