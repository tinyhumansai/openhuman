use super::*;

fn test_client() -> Arc<IntegrationClient> {
    Arc::new(IntegrationClient::new("http://test".into(), "tok".into()))
}

#[test]
fn search_tool_metadata_and_schema() {
    let tool = TinyFishSearchTool::new(test_client());
    assert_eq!(tool.name(), "tinyfish_search");
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert!(tool.description().contains("TinyFish"));

    let schema = tool.parameters_schema();
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "query"));
}

#[tokio::test]
async fn search_rejects_missing_query() {
    let tool = TinyFishSearchTool::new(test_client());
    assert!(tool.execute(json!({})).await.is_err());
}

#[tokio::test]
async fn search_rejects_empty_query() {
    let tool = TinyFishSearchTool::new(test_client());
    assert!(tool.execute(json!({"query": ""})).await.is_err());
}

#[test]
fn fetch_tool_metadata_and_schema() {
    let tool = TinyFishFetchTool::new(test_client());
    assert_eq!(tool.name(), "tinyfish_fetch");
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert!(tool.description().contains("JavaScript-heavy"));

    let schema = tool.parameters_schema();
    assert!(schema["properties"]["urls"].is_object());
    assert!(schema["properties"]["format"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "markdown"));
    assert!(schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "urls"));
}

#[tokio::test]
async fn fetch_rejects_empty_urls() {
    let tool = TinyFishFetchTool::new(test_client());
    let result = tool.execute(json!({"urls": []})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("at least one URL"));
}

#[tokio::test]
async fn fetch_rejects_non_string_url() {
    let tool = TinyFishFetchTool::new(test_client());
    let result = tool.execute(json!({"urls": [42]})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not a string"));
}

#[test]
fn agent_run_tool_metadata_and_schema() {
    let tool = TinyFishAgentRunTool::new(test_client());
    assert_eq!(tool.name(), "tinyfish_agent_run");
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    assert!(tool.description().contains("browser automation"));

    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "url"));
    assert!(required.iter().any(|v| v == "goal"));
    assert!(schema["properties"]["browser_profile"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "stealth"));
}

#[tokio::test]
async fn agent_run_rejects_missing_goal() {
    let tool = TinyFishAgentRunTool::new(test_client());
    assert!(tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .is_err());
}

#[tokio::test]
async fn agent_run_rejects_non_object_output_schema() {
    let tool = TinyFishAgentRunTool::new(test_client());
    let result = tool
        .execute(json!({
            "url": "https://example.com",
            "goal": "Return JSON",
            "output_schema": []
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("output_schema"));
}

#[test]
fn search_response_deserializes() {
    let json = r#"{
        "results": [
            {
                "position": 1,
                "site_name": "example.com",
                "title": "Example",
                "snippet": "Example snippet",
                "url": "https://example.com"
            }
        ],
        "total_results": 1,
        "costUsd": 0.0
    }"#;
    let resp: TinyFishSearchResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.results.len(), 1);
    assert_eq!(resp.results[0].title, "Example");
    assert_eq!(resp.total_results, Some(1));
}

#[test]
fn fetch_response_deserializes_with_errors() {
    let json = r##"{
        "results": [
            {
                "url": "https://example.com",
                "final_url": "https://example.com",
                "title": "Example",
                "text": "# Example",
                "links": ["https://example.com/a"],
                "image_links": ["https://example.com/a.png"],
                "latency_ms": 12
            }
        ],
        "errors": [
            {
                "url": "https://blocked.example",
                "error": "bot_blocked",
                "status": 403
            }
        ]
    }"##;
    let resp: TinyFishFetchResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.results.len(), 1);
    assert_eq!(resp.errors.len(), 1);
    assert_eq!(resp.errors[0].status, Some(403));
}

#[test]
fn agent_run_response_deserializes() {
    let json = r#"{
        "run_id": "run_123",
        "status": "COMPLETED",
        "num_of_steps": 4,
        "result": {"ok": true},
        "costUsd": 0.12
    }"#;
    let resp: TinyFishAgentRunResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.run_id.as_deref(), Some("run_123"));
    assert_eq!(resp.status, "COMPLETED");
    assert_eq!(resp.num_of_steps, Some(4));
    assert_eq!(resp.cost_usd, Some(0.12));
}

#[test]
fn agent_run_output_hides_internal_run_id() {
    let resp = TinyFishAgentRunResponse {
        run_id: Some("run_123".to_string()),
        status: "COMPLETED".to_string(),
        result: Some(json!({"ok": true})),
        error: None,
        num_of_steps: Some(4),
        cost_usd: Some(0.12),
    };

    let output = format_agent_run_response(resp);

    assert!(output.contains("TinyFish automation finished."));
    assert!(output.contains("Status: COMPLETED"));
    assert!(!output.contains("Run ID:"));
    assert!(!output.contains("run_123"));
}
