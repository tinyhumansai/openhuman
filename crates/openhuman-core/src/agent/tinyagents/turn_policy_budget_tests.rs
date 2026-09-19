use super::*;

#[tokio::test]
async fn per_turn_tool_limit_reaches_the_execution_policy() {
    crate::agent::stop_hooks::with_tool_call_limit(Some(2), async {
        let policy = run_policy_for(10, false);
        assert_eq!(policy.limits.max_tool_calls, 2);
        assert_eq!(policy.limits.max_model_calls, 10);
    })
    .await;
    assert_eq!(run_policy_for(10, false).limits.max_tool_calls, 80);
}
