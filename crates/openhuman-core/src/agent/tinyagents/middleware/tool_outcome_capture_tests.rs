use super::*;
use tinyagents_harness::context::{RunConfig, RunContext};

fn context() -> RunContext<crate::agent::tinyagents::host::OpenHumanRunContext> {
    RunContext::new(
        RunConfig::new("outcome-capture-test"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    )
}

#[tokio::test]
async fn same_tool_calls_keep_completion_and_failure_records_by_call_id() {
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let failure_map = std::sync::Arc::new(std::sync::Mutex::new(Default::default()));
    let middleware = ToolOutcomeCaptureMiddleware::new(sink.clone(), failure_map.clone());
    let mut ctx = context();

    let mut success_call = tinyinference_llm::tool::ToolCall {
        id: "echo-success".into(),
        name: "echo".into(),
        arguments: serde_json::json!({"message": "hello"}),
        invalid: None,
    };
    middleware
        .before_tool(&mut ctx, &(), &mut success_call)
        .await
        .expect("start metadata is captured");
    let success = ToolInvocationIdentity::new("echo-success", "echo");
    let mut success_result = TaToolResult::success("done");
    middleware
        .after_tool(&mut ctx, &(), &success, &mut success_result)
        .await
        .expect("successful result is captured");

    let mut failure_call = tinyinference_llm::tool::ToolCall {
        id: "echo-failure".into(),
        name: "echo".into(),
        arguments: serde_json::json!({"message": "retry"}),
        invalid: None,
    };
    middleware
        .before_tool(&mut ctx, &(), &mut failure_call)
        .await
        .expect("start metadata is captured");
    let failure = ToolInvocationIdentity::new("echo-failure", "echo");
    let mut failure_result = TaToolResult::error("request timed out");
    middleware
        .after_tool(&mut ctx, &(), &failure, &mut failure_result)
        .await
        .expect("failed result is captured");

    let outcomes = sink.lock().expect("outcome sink");
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].call_id, "echo-success");
    assert!(outcomes[0].success);
    assert_eq!(
        outcomes[0].arguments,
        serde_json::json!({"message": "hello"})
    );
    assert_eq!(outcomes[1].call_id, "echo-failure");
    assert!(!outcomes[1].success);
    assert_eq!(
        outcomes[1].arguments,
        serde_json::json!({"message": "retry"})
    );
    drop(outcomes);

    let recorded = failure_map.lock().expect("failure lookup");
    assert_eq!(recorded.len(), 2, "same tool names cannot overwrite calls");
    assert!(recorded["echo-success"].0);
    assert!(!recorded["echo-failure"].0);
    assert!(recorded["echo-failure"].1.is_some());
}

async fn capture_metadata(
    metadata: serde_json::Value,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let failure_map = std::sync::Arc::new(std::sync::Mutex::new(Default::default()));
    let middleware = ToolOutcomeCaptureMiddleware::new(sink, failure_map.clone());
    let mut ctx = context();
    let mut call = tinyinference_llm::tool::ToolCall {
        id: "mcp-1".into(),
        name: "mcp_shop_order".into(),
        arguments: serde_json::json!({}),
        invalid: None,
    };
    middleware
        .before_tool(&mut ctx, &(), &mut call)
        .await
        .unwrap();
    let identity = ToolInvocationIdentity::new("mcp-1", "mcp_shop_order");
    let mut result = TaToolResult::success("ok");
    result.metadata = Some(metadata);
    middleware
        .after_tool(&mut ctx, &(), &identity, &mut result)
        .await
        .unwrap();
    let structured = failure_map.lock().unwrap()["mcp-1"].4.clone();
    (structured, result.metadata)
}

#[tokio::test]
async fn mcp_ui_presentation_is_forwarded() {
    let (structured, kept) =
        capture_metadata(serde_json::json!({"kind": "mcp_ui", "tool": "order", "links": []})).await;
    assert_eq!(structured.unwrap()["kind"], "mcp_ui");
    assert!(kept.is_some());
}

#[tokio::test]
async fn raw_mcp_result_envelope_is_dropped() {
    let (structured, kept) = capture_metadata(serde_json::json!({
        "kind": "mcp_result",
        "server": "shop",
        "tool": "order",
        "resources": [{"uri": "ui://x", "mimeType": "text/html", "text": "<html>"}]
    }))
    .await;
    assert!(structured.is_none());
    assert!(kept.is_none());
}
