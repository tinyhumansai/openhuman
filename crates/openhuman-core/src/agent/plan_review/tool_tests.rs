use super::*;
use crate::agent::turn_origin::with_origin;
use crate::security::approval::ApprovalChatContext;

/// The execution context the harness hands a tool dispatched inside a turn
/// whose chat is `chat`.
fn chat_tool_context(
    chat: crate::security::approval::ApprovalChatContext,
) -> tinyagents_harness::tool::ToolExecutionContext {
    crate::agent::tinyagents::host::run_context::tool_context_with_approval(
        crate::security::approval::ApprovalTurnContext::chat(chat),
    )
}

#[tokio::test]
async fn non_interactive_origin_auto_approves() {
    let tool = RequestPlanReviewTool::new();
    let out = with_origin(
        AgentTurnOrigin::Cli,
        tool.execute(json!({ "summary": "do x", "steps": ["a", "b"] })),
    )
    .await
    .unwrap();
    assert!(!out.is_error);
    assert!(out.output().starts_with("approved"));
}

#[tokio::test]
async fn interactive_turn_parks_until_resolved() {
    // The tool is inert (no-ops rather than parks) outside Plan mode — see
    // `plan_mode_is_inert_outside_plan_mode` below — so this thread must be
    // in Plan mode for the park to actually happen.
    crate::agent::tinyagents::run_mode::set_mode(
        &crate::core::runtime::DispatchCtx::ambient(),
        "t-int",
        tinyagents_harness::middleware::RunMode::Plan,
    );
    let tool = RequestPlanReviewTool::new();
    let context = chat_tool_context(ApprovalChatContext {
        thread_id: "t-int".into(),
        client_id: "c-int".into(),
        request_id: Some("req-int".into()),
    });
    let fut = with_origin(
        AgentTurnOrigin::WebChat {
            thread_id: "t-int".into(),
            client_id: "c-int".into(),
            request_id: Some("req-int".into()),
        },
        tool.execute_with_context(
            json!({ "summary": "plan", "steps": ["one"] }),
            tinytools::ToolCallOptions::default(),
            Some(&context),
        ),
    );
    // The web channel supplies the origin and the chat context. One poll reaches the gate
    // and must remain pending; no wall-clock timeout is needed to prove park.
    tokio::pin!(fut);
    assert!(
        matches!(futures::poll!(fut.as_mut()), std::task::Poll::Pending),
        "interactive turn should park, not resolve immediately"
    );
}

#[tokio::test]
async fn plan_mode_is_inert_outside_plan_mode() {
    // A thread that has never toggled plan mode defaults to Build — the
    // tool must no-op (return immediately) rather than park.
    let tool = RequestPlanReviewTool::new();
    let out = with_origin(
        AgentTurnOrigin::WebChat {
            thread_id: "t-build-inert".into(),
            client_id: "c-build-inert".into(),
            request_id: Some("req-build-inert".into()),
        },
        tool.execute(json!({ "summary": "plan", "steps": ["one"] })),
    )
    .await
    .unwrap();
    assert!(!out.is_error);
    assert!(out.output().starts_with("not applicable"));
}
