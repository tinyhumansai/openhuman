use super::*;
use tinyagents_harness::context::{RunConfig, RunContext};

fn context() -> RunContext<crate::agent::tinyagents::host::OpenHumanRunContext> {
    RunContext::new(
        RunConfig::new("outcome-capture-test"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    )
}

#[tokio::test]
async fn same_tool_calls_keep_completion_and_failure_records_by_call_id() {
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let failure_map = std::sync::Arc::new(std::sync::Mutex::new(Default::default()));
    let middleware = ToolOutcomeCaptureMiddleware::new(sink.clone(), failure_map.clone());
    let mut ctx = context();

    let success = ToolInvocationIdentity::new("echo-success", "echo");
    let mut success_result = TaToolResult::success("done");
    middleware
        .after_tool(&mut ctx, &(), &success, &mut success_result)
        .await
        .expect("successful result is captured");

    let failure = ToolInvocationIdentity::new("echo-failure", "echo");
    let mut failure_result = TaToolResult::error("request timed out");
    middleware
        .after_tool(&mut ctx, &(), &failure, &mut failure_result)
        .await
        .expect("failed result is captured");

    let outcomes = sink.lock().expect("outcome sink");
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].call_id, "echo-success");
    assert!(outcomes[0].success);
    assert_eq!(outcomes[1].call_id, "echo-failure");
    assert!(!outcomes[1].success);
    drop(outcomes);

    let recorded = failure_map.lock().expect("failure lookup");
    assert_eq!(recorded.len(), 2, "same tool names cannot overwrite calls");
    assert!(recorded["echo-success"].0);
    assert!(!recorded["echo-failure"].0);
    assert!(recorded["echo-failure"].1.is_some());
}
