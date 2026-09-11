use super::*;

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
