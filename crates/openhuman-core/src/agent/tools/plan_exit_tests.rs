use crate::agent::tools::{plan_exit::plan_exit_tool, PLAN_EXIT_MARKER};
use serde_json::json;
use tinyagents_harness_tools::plan_review::{PlanReviewRequest, PlanReviewResolution};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolRunContext};

struct RunModeEventCollector(tokio::sync::mpsc::UnboundedSender<(String, String)>);

#[async_trait::async_trait]
impl tinybus::EventHandler<crate::core::events::DomainEvent> for RunModeEventCollector {
    fn name(&self) -> &str {
        "plan-exit-test"
    }

    async fn handle(&self, event: &crate::core::events::DomainEvent) {
        if let crate::core::events::DomainEvent::ThreadRunModeChanged { thread_id, mode } = event {
            let _ = self.0.send((thread_id.clone(), mode.clone()));
        }
    }
}

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
    crate::threads::run_mode::set_mode(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread_id,
        tinyagents_harness::middleware::RunMode::Plan,
    );
    let store = crate::plan_review::host::global();
    let tenant = crate::core::runtime::DispatchCtx::ambient()
        .tenant()
        .unwrap()
        .profile_only();
    let parked = tokio::spawn(async move {
        store
            .request_review(
                tenant,
                thread_id.to_string(),
                None,
                PlanReviewRequest {
                    summary: "Plan".into(),
                    steps: vec!["step".into()],
                    tool_call_id: None,
                },
            )
            .await
    });
    // Let the review register as parked before racing plan_exit against it.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let tool = plan_exit_tool();
    let ctx = ThreadContext(thread_id);
    let result = tool
        .execute_with_context(
            json!({ "plan": "1. Do the thing" }),
            ToolCallOptions::default(),
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(result.is_error, "a parked review must refuse plan exit");
    assert_eq!(
        crate::threads::run_mode::get_mode(
            &crate::core::runtime::DispatchCtx::ambient(),
            thread_id
        ),
        tinyagents_harness::middleware::RunMode::Plan,
        "plan_exit must not flip to Build while a review is still parked"
    );

    // Resolve the parked review so the spawned task and the gate's
    // bookkeeping don't leak past this test.
    assert!(crate::plan_review::host::global().decide_by_thread(
        &crate::core::runtime::DispatchCtx::ambient()
            .tenant()
            .unwrap()
            .profile_only(),
        thread_id,
        PlanReviewResolution::Approve,
    ));
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
        crate::threads::run_mode::get_mode(
            &crate::core::runtime::DispatchCtx::ambient(),
            thread_id
        ),
        tinyagents_harness::middleware::RunMode::Build
    );
}

#[tokio::test]
async fn plan_exit_emits_marker() {
    let tool = plan_exit_tool();
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
async fn plan_exit_publishes_build_run_mode_change() {
    crate::core::bus::init().await.expect("bus init");
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let _subscription = crate::core::bus::BUS
        .subscribe(std::sync::Arc::new(RunModeEventCollector(tx)))
        .expect("subscribe to core events");
    let thread_id = "plan-exit-publishes-run-mode";
    crate::threads::run_mode::set_mode(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread_id,
        tinyagents_harness::middleware::RunMode::Plan,
    );
    let tool = plan_exit_tool();
    let result = tool
        .execute_with_context(
            json!({ "plan": "1. Do the thing" }),
            ToolCallOptions::default(),
            Some(&ThreadContext(thread_id)),
        )
        .await
        .unwrap();
    assert!(!result.is_error);

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match events.recv().await {
                Some((event_thread_id, mode))
                    if event_thread_id == thread_id && mode == "build" =>
                {
                    break (event_thread_id, mode);
                }
                Some(_) => continue,
                None => panic!("run mode event stream ended"),
            }
        }
    })
    .await
    .expect("plan_exit must publish run_mode_changed");
    assert_eq!(event, (thread_id.into(), "build".into()));
}

#[tokio::test]
async fn plan_exit_rejects_empty() {
    let tool = plan_exit_tool();
    let result = tool.execute(json!({ "plan": "   " })).await.unwrap();
    assert!(result.is_error);
}

#[test]
fn plan_exit_metadata() {
    let tool = plan_exit_tool();
    assert_eq!(tool.name(), "plan_exit");
    assert_eq!(tool.permission_level(), PermissionLevel::None);
}
