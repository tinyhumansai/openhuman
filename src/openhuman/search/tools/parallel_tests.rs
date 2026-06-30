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

#[test]
fn research_output_hides_internal_run_id() {
    let resp = ResearchResponse {
        run_id: Some("run_internal_123".into()),
        status: Some("completed".into()),
        result: Some(json!({ "summary": "useful answer" })),
        cost_usd: 0.1234,
    };
    let output = format_research_response(ResearchResponse {
        run_id: resp.run_id.clone(),
        status: resp.status.clone(),
        result: resp.result.clone(),
        cost_usd: resp.cost_usd,
    })
    .unwrap();
    let payload = research_payload(&resp, &output);

    assert!(output.contains("Status: completed"));
    assert!(output.contains("useful answer"));
    assert!(output.contains("Cost: $0.1234"));
    assert!(!output.contains("Run:"));
    assert!(!output.contains("run_internal_123"));
    assert!(payload.get("run_id").is_none());
}

#[test]
fn enrich_output_hides_internal_run_id() {
    let resp = EnrichResponse {
        run_id: Some("run_internal_456".into()),
        status: Some("completed".into()),
        output: Some(json!({ "company": "OpenHuman" })),
        cost_usd: 0.5678,
    };
    let output = format_enrich_response(EnrichResponse {
        run_id: resp.run_id.clone(),
        status: resp.status.clone(),
        output: resp.output.clone(),
        cost_usd: resp.cost_usd,
    })
    .unwrap();
    let payload = enrich_payload(&resp, &output);

    assert!(output.contains("Status: completed"));
    assert!(output.contains("OpenHuman"));
    assert!(output.contains("Cost: $0.5678"));
    assert!(!output.contains("Run:"));
    assert!(!output.contains("run_internal_456"));
    assert!(payload.get("run_id").is_none());
}

#[test]
fn research_incomplete_response_returns_actionable_error_without_run_id() {
    let err = format_research_response(ResearchResponse {
        run_id: Some("run_internal_789".into()),
        status: Some("running".into()),
        result: None,
        cost_usd: 0.1111,
    })
    .unwrap_err();

    assert!(err.contains("did not return a result"));
    assert!(err.contains("higher timeout_seconds"));
    assert!(!err.contains("run_internal_789"));
}

#[test]
fn enrich_incomplete_response_returns_actionable_error_without_run_id() {
    let err = format_enrich_response(EnrichResponse {
        run_id: Some("run_internal_987".into()),
        status: Some("running".into()),
        output: None,
        cost_usd: 0.2222,
    })
    .unwrap_err();

    assert!(err.contains("did not return output"));
    assert!(err.contains("higher timeout_seconds"));
    assert!(!err.contains("run_internal_987"));
}
