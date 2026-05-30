use super::*;
use crate::openhuman::integrations::ToolScope;

fn test_client() -> Arc<IntegrationClient> {
    Arc::new(IntegrationClient::new("http://test".into(), "tok".into()))
}

// ── ParallelSearchTool ──────────────────────────────────────────

#[test]
fn search_tool_metadata() {
    let tool = ParallelSearchTool::new(test_client());
    assert_eq!(tool.name(), "parallel_search");
    assert_eq!(tool.scope(), ToolScope::All);
    assert!(tool.description().contains("web search"));
}

#[test]
fn search_schema_required_fields() {
    let tool = ParallelSearchTool::new(test_client());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "objective"));
    assert!(required.iter().any(|v| v == "search_queries"));
}

#[tokio::test]
async fn search_rejects_missing_objective() {
    let tool = ParallelSearchTool::new(test_client());
    assert!(tool
        .execute(json!({"search_queries": ["test"]}))
        .await
        .is_err());
}

#[tokio::test]
async fn search_rejects_empty_objective() {
    let tool = ParallelSearchTool::new(test_client());
    let result = tool
        .execute(json!({"objective": "", "search_queries": ["test"]}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn search_rejects_empty_queries() {
    let tool = ParallelSearchTool::new(test_client());
    let result = tool
        .execute(json!({"objective": "test", "search_queries": []}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[test]
fn search_response_rejects_missing_search_id() {
    let json = r#"{
        "results": [],
        "costUsd": 0.01
    }"#;
    assert!(serde_json::from_str::<SearchResponse>(json).is_err());
}

#[test]
fn search_response_rejects_missing_results() {
    let json = r#"{
        "searchId": "s123",
        "costUsd": 0.01
    }"#;
    assert!(serde_json::from_str::<SearchResponse>(json).is_err());
}

#[test]
fn search_response_rejects_missing_cost_usd() {
    let json = r#"{
        "searchId": "s123",
        "results": []
    }"#;
    assert!(serde_json::from_str::<SearchResponse>(json).is_err());
}

#[test]
fn search_response_deserializes() {
    let json = r#"{
        "searchId": "s123",
        "results": [
            {
                "url": "https://example.com",
                "title": "Example",
                "publish_date": "2026-01-01",
                "excerpts": ["Some text"]
            }
        ],
        "costUsd": 0.01
    }"#;
    let resp: SearchResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.results.len(), 1);
    assert_eq!(resp.results[0].title, "Example");
}

// ── ParallelExtractTool ─────────────────────────────────────────

#[test]
fn extract_tool_metadata() {
    let tool = ParallelExtractTool::new(test_client());
    assert_eq!(tool.name(), "parallel_extract");
    assert_eq!(tool.scope(), ToolScope::All);
    assert!(tool.description().contains("Extract content"));
}

#[test]
fn extract_schema_required_urls() {
    let tool = ParallelExtractTool::new(test_client());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "urls"));
}

#[tokio::test]
async fn extract_rejects_missing_urls() {
    let tool = ParallelExtractTool::new(test_client());
    assert!(tool.execute(json!({})).await.is_err());
}

#[tokio::test]
async fn extract_rejects_empty_urls() {
    let tool = ParallelExtractTool::new(test_client());
    let result = tool.execute(json!({"urls": []})).await.unwrap();
    assert!(result.is_error);
}

#[test]
fn extract_response_deserializes() {
    let json = r#"{
        "extractId": "e123",
        "results": [
            {
                "url": "https://example.com",
                "title": "Example Page",
                "excerpts": ["Key info here"],
                "full_content": null
            }
        ],
        "errors": [
            {"url": "https://bad.com", "error": "timeout"}
        ],
        "costUsd": 0.002
    }"#;
    let resp: ExtractResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.results.len(), 1);
    assert_eq!(resp.errors.len(), 1);
    assert_eq!(resp.errors[0].url, "https://bad.com");
}

#[test]
fn extract_response_with_full_content() {
    let json = r#"{
        "extractId": "e456",
        "results": [
            {
                "url": "https://example.com",
                "title": "Full Article",
                "excerpts": [],
                "full_content": "This is the full article content."
            }
        ],
        "errors": [],
        "costUsd": 0.002
    }"#;
    let resp: ExtractResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        resp.results[0].full_content.as_deref(),
        Some("This is the full article content.")
    );
}
