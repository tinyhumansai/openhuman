use super::*;
use tinyagents_harness::context::RunConfig;
use tinyinference::tool::ToolSchema;

#[tokio::test]
async fn live_request_and_execution_share_the_fail_closed_allowlist() {
    let candidates = vec!["web_fetch".to_string(), "file_write".to_string()];
    let allowed = std::collections::HashSet::from(["web_fetch".to_string()]);
    let middleware = OpenHumanToolExposureMiddleware::new(&candidates, Some(&allowed), Vec::new());
    let mut ctx = RunContext::new(RunConfig::new("tool-exposure-test"), ());
    let mut request = ModelRequest {
        tools: candidates
            .iter()
            .map(|name| ToolSchema::new(name, "", serde_json::json!({})))
            .collect(),
        ..Default::default()
    };

    middleware
        .before_model(&mut ctx, &(), &mut request)
        .await
        .unwrap();
    assert_eq!(request.tools.len(), 1);
    assert_eq!(request.tools[0].name, "web_fetch");

    let mut hidden = TaToolCall::new("call-1", "file_write", serde_json::json!({}));
    assert!(middleware
        .before_tool(&mut ctx, &(), &mut hidden)
        .await
        .is_err());
}
