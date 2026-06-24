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
    assert_eq!(tool.category(), ToolCategory::Workflow);
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
    assert_eq!(tool.category(), ToolCategory::Workflow);
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
fn run_actor_output_hides_internal_ids() {
    let resp = ApifyRunResponse {
        run_id: "run-123".to_string(),
        actor_id: "apify/web-scraper".to_string(),
        status: "SUCCEEDED".to_string(),
        dataset_id: Some("dataset-123".to_string()),
        items: Some(vec![json!({"url": "https://example.com"})]),
        cost_usd: 0.3,
    };

    let output = format_run_actor_response(&resp, true);

    assert!(output.contains("Apify run started for actor: apify/web-scraper"));
    assert!(output.contains("Status: SUCCEEDED"));
    assert!(!output.contains("Run ID:"));
    assert!(!output.contains("Dataset ID:"));
    assert!(!output.contains("run-123"));
    assert!(!output.contains("dataset-123"));
}

#[test]
fn run_actor_async_output_shows_polling_instruction() {
    let resp = ApifyRunResponse {
        run_id: "run-456".to_string(),
        actor_id: "apify/crawler".to_string(),
        status: "RUNNING".to_string(),
        dataset_id: None,
        items: None,
        cost_usd: 0.05,
    };

    let output = format_run_actor_response(&resp, false);
    let prose = output
        .split("[apify_run_ref]")
        .next()
        .expect("prose before ref");

    assert!(output.contains("This run is still in progress. Poll with apify_get_run_status."));
    assert!(!prose.contains("run-456"));
    assert!(output.contains("[apify_run_ref]"));
    assert!(output.contains("\"run_id\":\"run-456\""));
}

#[test]
fn run_actor_sync_without_items_shows_follow_up_ref() {
    let resp = ApifyRunResponse {
        run_id: "run-789".to_string(),
        actor_id: "apify/crawler".to_string(),
        status: "RUNNING".to_string(),
        dataset_id: Some("dataset-789".to_string()),
        items: None,
        cost_usd: 0.05,
    };

    let output = format_run_actor_response(&resp, true);
    let prose = output
        .split("[apify_run_ref]")
        .next()
        .expect("prose before ref");

    assert!(output.contains("Poll with apify_get_run_status"));
    assert!(!prose.contains("run-789"));
    assert!(output.contains("[apify_run_ref]"));
    assert!(output.contains("\"run_id\":\"run-789\""));
    assert!(output.contains("\"dataset_id\":\"dataset-789\""));
}

#[test]
fn run_status_output_hides_internal_ids() {
    let resp = ApifyRunResponse {
        run_id: "run-123".to_string(),
        actor_id: "apify/web-scraper".to_string(),
        status: "RUNNING".to_string(),
        dataset_id: Some("dataset-123".to_string()),
        items: None,
        cost_usd: 0.1,
    };

    let output = format_run_status_response(&resp);
    let prose = output
        .split("[apify_run_ref]")
        .next()
        .expect("prose before ref");

    assert!(output.contains("Actor ID: apify/web-scraper"));
    assert!(output.contains("Status: RUNNING"));
    assert!(!prose.contains("Run ID:"));
    assert!(!prose.contains("Dataset ID:"));
    assert!(!prose.contains("run-123"));
    assert!(!prose.contains("dataset-123"));
    assert!(output.contains("[apify_run_ref]"));
    assert!(output.contains("\"run_id\":\"run-123\""));
    assert!(output.contains("\"dataset_id\":\"dataset-123\""));
}

#[test]
fn results_response_deserializes() {
    let json = r#"{"items":[{"foo":"bar"}],"total":42}"#;
    let resp: ApifyGetRunResultsResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.items.len(), 1);
    assert_eq!(resp.total, 42);
}
