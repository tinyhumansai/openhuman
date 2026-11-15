use super::*;
use serde_json::json;

#[test]
fn classified_result_maps_rendered_content_and_preserves_outcome_metadata() {
    let call = tinymcp_bus::ClassifiedStaticToolCall {
        outcome: tinymcp_bus::McpCallOutcome::answered("docs", "search"),
        rendered: Some(tinymcp_bus::McpToolResult {
            content: vec![
                tinymcp_bus::McpToolContent::Text {
                    text: "matched".to_owned(),
                },
                tinymcp_bus::McpToolContent::Json {
                    data: json!({ "count": 1 }),
                },
            ],
            ..Default::default()
        }),
        envelope: None,
        error_message: None,
    };

    let result = classified_result(call, "docs", "search", true);

    assert!(!result.is_error);
    assert!(result.output().contains("matched"));
    assert_eq!(
        result.markdown_formatted.as_deref(),
        Some(result.output().as_str())
    );
    assert_eq!(
        tinymcp_bus::McpCallOutcome::from_metadata(result.metadata.as_ref().unwrap()),
        Some(tinymcp_bus::McpCallOutcome::answered("docs", "search"))
    );
}

#[tokio::test]
async fn disabled_module_returns_a_structured_unavailable_outcome() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let tool = mcp_call_tool(Arc::new(config));

    let result = tool
        .execute(json!({
            "server": "docs",
            "tool": "search",
            "arguments": {}
        }))
        .await
        .expect("tool failures are returned as a model-visible result");

    assert!(result.is_error);
    let metadata = result.metadata.as_ref().expect("call outcome is retained");
    let outcome = tinymcp_bus::McpCallOutcome::from_metadata(metadata).unwrap();
    assert_eq!(outcome.server, "docs");
    assert_eq!(outcome.tool, "search");
    assert_eq!(outcome.error.unwrap().code, tinymcp_bus::errors::BUS);
}
