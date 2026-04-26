use super::*;

fn test_client() -> Arc<IntegrationClient> {
    Arc::new(IntegrationClient::new(
        "http://test.example".into(),
        "tok".into(),
    ))
}

#[test]
fn run_tool_metadata() {
    let tool = ApifyRunActorTool::new(test_client());
    assert_eq!(tool.name(), "apify_run_actor");
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    assert_eq!(tool.category(), ToolCategory::Skill);
    assert!(tool.description().contains("Apify actor"));
}

#[test]
fn run_tool_schema_has_required_fields() {
    let tool = ApifyRunActorTool::new(test_client());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "actor_id"));
    assert!(required.iter().any(|v| v == "input"));
}

#[tokio::test]
async fn run_tool_rejects_missing_actor_id() {
    let tool = ApifyRunActorTool::new(test_client());
    let result = tool.execute(json!({"input": {}})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn run_tool_rejects_empty_actor_id() {
    let tool = ApifyRunActorTool::new(test_client());
    let result = tool
        .execute(json!({"actor_id": "", "input": {}}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("actor_id"));
}

#[tokio::test]
async fn run_tool_rejects_non_object_input() {
    let tool = ApifyRunActorTool::new(test_client());
    let result = tool
        .execute(json!({"actor_id": "apify/web-scraper", "input": []}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("input must be a JSON object"));
}

#[test]
fn status_tool_metadata() {
    let tool = ApifyGetRunStatusTool::new(test_client());
    assert_eq!(tool.name(), "apify_get_run_status");
    assert_eq!(tool.category(), ToolCategory::Skill);
}

#[tokio::test]
async fn status_tool_rejects_empty_run_id() {
    let tool = ApifyGetRunStatusTool::new(test_client());
    let result = tool.execute(json!({"run_id": ""})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("run_id"));
}

#[test]
fn results_tool_schema_supports_pagination() {
    let tool = ApifyGetRunResultsTool::new(test_client());
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["limit"].is_object());
    assert!(schema["properties"]["offset"].is_object());
}

#[tokio::test]
async fn results_tool_rejects_empty_run_id() {
    let tool = ApifyGetRunResultsTool::new(test_client());
    let result = tool.execute(json!({"run_id": ""})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("run_id"));
}

#[test]
fn run_response_deserializes() {
    let json = r#"{
        "runId":"run-123",
        "actorId":"apify/web-scraper",
        "status":"SUCCEEDED",
        "datasetId":"dataset-123",
        "items":[{"url":"https://example.com"}],
        "costUsd":0.3
    }"#;
    let resp: ApifyRunResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.run_id, "run-123");
    assert_eq!(resp.actor_id, "apify/web-scraper");
    assert_eq!(resp.status, "SUCCEEDED");
    assert_eq!(resp.dataset_id.as_deref(), Some("dataset-123"));
    assert_eq!(resp.items.unwrap().len(), 1);
    assert!((resp.cost_usd - 0.3).abs() < f64::EPSILON);
}

#[test]
fn results_response_deserializes() {
    let json = r#"{"items":[{"foo":"bar"}],"total":42}"#;
    let resp: ApifyGetRunResultsResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.items.len(), 1);
    assert_eq!(resp.total, 42);
}
