use super::*;
use serde_json::json;

fn moved(arguments: serde_json::Value) -> serde_json::Value {
    let mut arguments = arguments.as_object().cloned().unwrap();
    timeout_to_yield(&mut arguments);
    serde_json::Value::Object(arguments)
}

#[test]
fn the_models_timeout_becomes_the_yield_point() {
    assert_eq!(
        moved(json!({"command": "make", "timeout_secs": 300})),
        json!({"command": "make", YIELD_AFTER_ARG: 300})
    );
}

#[test]
fn a_call_without_a_timeout_may_yield_at_the_default_point() {
    assert_eq!(
        moved(json!({"command": "ls"})),
        json!({"command": "ls", YIELD_AFTER_ARG: null})
    );
}

#[test]
fn zero_still_means_wait_for_the_end() {
    assert_eq!(
        moved(json!({"command": "make", "timeout_secs": 0})),
        json!({"command": "make", YIELD_AFTER_ARG: 0})
    );
}

#[test]
fn the_turn_clamp_then_sets_the_deadline() {
    use std::time::Duration;
    use tinyagents_harness::middleware::clamp_shell_timeout;

    // With the model's value moved aside, the clamp sees a call that asked for
    // no deadline and gives it the turn's remainder (less its reserve).
    let arguments = moved(json!({"command": "make", "timeout_secs": 300}));
    let requested = arguments.get("timeout_secs").and_then(|v| v.as_u64());
    let clamp = clamp_shell_timeout(requested, Duration::from_secs(600), 3600).unwrap();
    assert_eq!(clamp.requested, None);
    assert_eq!(clamp.clamped_secs, 510);
}

#[tokio::test]
async fn a_turn_that_cannot_follow_a_process_leaves_the_kill() {
    use tinyagents_harness::context::RunConfig;
    let mut ctx = OpenHumanRunContext::new().into_tinyagents(RunConfig::new("shell-yield"));
    let mut call = TaToolCall {
        id: "c1".into(),
        name: "shell".into(),
        arguments: json!({"command": "make", "timeout_secs": 60}),
        invalid: None,
    };
    ShellYieldMiddleware { can_follow: false }
        .before_tool(&mut ctx, &(), &mut call)
        .await
        .unwrap();
    assert_eq!(
        call.arguments,
        json!({"command": "make", "timeout_secs": 60})
    );

    ShellYieldMiddleware { can_follow: true }
        .before_tool(&mut ctx, &(), &mut call)
        .await
        .unwrap();
    assert_eq!(
        call.arguments,
        json!({"command": "make", YIELD_AFTER_ARG: 60})
    );
}
