use super::*;
use crate::openhuman::agent::turn_origin::with_origin;

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
    let tool = RequestPlanReviewTool::new();
    let fut = with_origin(
        AgentTurnOrigin::WebChat {
            thread_id: "t-int".into(),
            client_id: "c-int".into(),
            request_id: Some("req-int".into()),
        },
        tool.execute(json!({ "summary": "plan", "steps": ["one"] })),
    );
    // An interactive turn must BLOCK on the gate rather than return
    // immediately — a short timeout elapses with no result (the parked
    // future is then dropped, and the gate cleans up).
    let res = tokio::time::timeout(std::time::Duration::from_millis(60), fut).await;
    assert!(
        res.is_err(),
        "interactive turn should park, not resolve immediately"
    );
}
