use super::*;

#[test]
fn schema_requires_task_id_and_message() {
    let schema = SteerSubagentTool::new().parameters_schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required list");
    assert!(required.iter().any(|v| v.as_str() == Some("message")));
}

#[tokio::test]
async fn missing_task_id_is_rejected() {
    let tool = SteerSubagentTool::new();
    let res = tool.execute(json!({ "message": "go" })).await.unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("subagent_session_id"));
}

#[tokio::test]
async fn missing_message_is_rejected() {
    let tool = SteerSubagentTool::new();
    let res = tool.execute(json!({ "task_id": "sub-1" })).await.unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("message"));
}

#[tokio::test]
async fn outside_agent_turn_is_rejected() {
    // No PARENT_CONTEXT task-local installed in a bare test.
    let tool = SteerSubagentTool::new();
    let res = tool
        .execute(json!({ "task_id": "sub-1", "message": "go" }))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("outside of an agent turn"));
}
