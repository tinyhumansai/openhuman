use super::*;
use crate::agent::plan_review::PlanReviewResolution;

struct ThreadContext(&'static str);
impl ToolRunContext for ThreadContext {
    fn thread_id(&self) -> Option<&str> {
        Some(self.0)
    }
}

/// `plan_exit` must not flip a thread to Build while a plan review is still
/// parked on it — flipping early would unlock every tool before the user
/// actually approved anything. Uses the process-global plan-review gate
/// (the same one `plan_exit` reads) parked on a dedicated thread id so this
/// test doesn't collide with others sharing the singleton.
#[tokio::test]
async fn plan_exit_does_not_flip_while_a_review_is_parked() {
    let thread_id = "plan-exit-parked-thread";
    crate::agent::tinyagents::run_mode::set_mode(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread_id,
        tinyagents_harness::middleware::RunMode::Plan,
    );
    let gate = crate::agent::plan_review::gate::global();
    let parked = tokio::spawn(async move {
        gate.request_review(
            Some(thread_id.to_string()),
            None,
            "Plan".into(),
            vec!["step".into()],
            None,
        )
        .await
    });
    // Let the review register as parked before racing plan_exit against it.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let tool = PlanExitTool::new();
    let ctx = ThreadContext(thread_id);
    let result = tool
        .execute_with_context(
            json!({ "plan": "1. Do the thing" }),
            ToolCallOptions::default(),
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(
        crate::agent::tinyagents::run_mode::get_mode(
            &crate::core::runtime::DispatchCtx::ambient(),
            thread_id
        ),
        tinyagents_harness::middleware::RunMode::Plan,
        "plan_exit must not flip to Build while a review is still parked"
    );

    // Resolve the parked review so the spawned task and the gate's
    // bookkeeping don't leak past this test.
    assert!(crate::agent::plan_review::gate::global()
        .decide_by_thread(thread_id, PlanReviewResolution::Approve));
    let resolution = parked.await.unwrap();
    assert_eq!(resolution, PlanReviewResolution::Approve);

    // Now that the review is resolved (no longer parked), plan_exit flips.
    let result = tool
        .execute_with_context(
            json!({ "plan": "1. Do the thing" }),
            ToolCallOptions::default(),
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(
        crate::agent::tinyagents::run_mode::get_mode(
            &crate::core::runtime::DispatchCtx::ambient(),
            thread_id
        ),
        tinyagents_harness::middleware::RunMode::Build
    );
}

#[tokio::test]
async fn plan_exit_emits_marker() {
    let tool = PlanExitTool::new();
    let result = tool
        .execute(json!({ "plan": "1. Read X\n2. Edit Y" }))
        .await
        .unwrap();
    assert!(!result.is_error);
    let output = result.output();
    assert!(output.starts_with(PLAN_EXIT_MARKER));
    assert!(output.contains("Read X"));
}

#[tokio::test]
async fn plan_exit_rejects_empty() {
    let tool = PlanExitTool::new();
    let result = tool.execute(json!({ "plan": "   " })).await.unwrap();
    assert!(result.is_error);
}

#[test]
fn plan_exit_metadata() {
    let tool = PlanExitTool::new();
    assert_eq!(tool.name(), "plan_exit");
    assert_eq!(tool.permission_level(), PermissionLevel::None);
}
