use super::*;
use crate::agent::tinyagents::middleware::repeated_failure::RepeatedToolFailureMiddleware;

// A tool that fails because inference itself could not run (out of credits, a
// rejected provider config) halts the turn on the first failure with a typed
// cause, quoting the raw error only up to the halt bound.

#[tokio::test]
async fn an_out_of_credits_tool_failure_halts_with_bounded_copy_and_a_typed_cause() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut context = ctx();
    let padding = "x".repeat(5_000);
    let error = format!(
        "Sub-agent inference failed and did not complete: 402 Insufficient budget {padding}"
    );

    let mut call = TaToolCall::new("credits-1", "delegate_task", serde_json::json!({}));
    mw.before_tool(&mut context, &(), &mut call).await.unwrap();
    let mut result = failing_result("delegate_task", &error);
    mw.after_tool(
        &mut context,
        &(),
        &invocation("credits-1", "delegate_task"),
        &mut result,
    )
    .await
    .unwrap();

    assert_eq!(drain_pause_count(&handle), 1, "halts on the first failure");
    let summary = slot.lock().unwrap().clone().expect("a halt summary");
    let start = summary
        .find("Sub-agent inference failed")
        .expect("the summary quotes the error");
    let end = summary[start..]
        .find("\n… [truncated]")
        .map(|offset| start + offset)
        .expect("the quoted error is cut at the halt bound");
    let quoted = summary[start..end].chars().count();
    assert!(
        quoted <= 600,
        "the quoted error runs {quoted} chars, past the 600-char bound"
    );

    assert_eq!(
        context
            .take_halted_by_guard_cause()
            .map(|cause| cause.class),
        Some("out_of_credits".to_string())
    );
}
