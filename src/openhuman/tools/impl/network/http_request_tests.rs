use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_tool(allowed_domains: Vec<&str>) -> HttpRequestTool {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    });
    HttpRequestTool::new(
        security,
        allowed_domains.into_iter().map(String::from).collect(),
        1_000_000,
        30,
    )
}

#[test]
fn validate_accepts_valid_methods() {
    let tool = test_tool(vec!["example.com"]);
    assert!(tool.validate_method("GET").is_ok());
    assert!(tool.validate_method("POST").is_ok());
    assert!(tool.validate_method("PUT").is_ok());
    assert!(tool.validate_method("DELETE").is_ok());
    assert!(tool.validate_method("PATCH").is_ok());
    assert!(tool.validate_method("HEAD").is_ok());
    assert!(tool.validate_method("OPTIONS").is_ok());
}

#[test]
fn validate_rejects_invalid_method() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool.validate_method("INVALID").unwrap_err().to_string();
    assert!(err.contains("Unsupported HTTP method"));
}

#[tokio::test]
async fn execute_blocks_readonly_mode() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = HttpRequestTool::new(security, vec!["example.com".into()], 1_000_000, 30);
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
}

#[tokio::test]
async fn execute_blocks_when_rate_limited() {
    let security = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let tool = HttpRequestTool::new(security, vec!["example.com".into()], 1_000_000, 30);
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("rate limit"));
}

#[test]
fn truncate_response_within_limit() {
    let tool = test_tool(vec!["example.com"]);
    let text = "hello world";
    assert_eq!(tool.truncate_response(text), "hello world");
}

#[test]
fn truncate_response_over_limit() {
    let tool = HttpRequestTool::new(
        Arc::new(SecurityPolicy::default()),
        vec!["example.com".into()],
        10,
        30,
    );
    let text = "hello world this is long";
    let truncated = tool.truncate_response(text);
    assert!(truncated.len() <= 10 + 60);
    assert!(truncated.contains("[Response truncated"));
}

#[test]
fn parse_headers_preserves_original_values() {
    let tool = test_tool(vec!["example.com"]);
    let headers = json!({
        "Authorization": "Bearer secret",
        "Content-Type": "application/json",
        "X-API-Key": "my-key"
    });
    let parsed = tool.parse_headers(&headers);
    assert_eq!(parsed.len(), 3);
    assert!(parsed
        .iter()
        .any(|(k, v)| k == "Authorization" && v == "Bearer secret"));
    assert!(parsed
        .iter()
        .any(|(k, v)| k == "X-API-Key" && v == "my-key"));
    assert!(parsed
        .iter()
        .any(|(k, v)| k == "Content-Type" && v == "application/json"));
}

#[test]
fn redact_headers_for_display_redacts_sensitive() {
    let headers = vec![
        ("Authorization".into(), "Bearer secret".into()),
        ("Content-Type".into(), "application/json".into()),
        ("X-API-Key".into(), "my-key".into()),
        ("X-Secret-Token".into(), "tok-123".into()),
    ];
    let redacted = HttpRequestTool::redact_headers_for_display(&headers);
    assert_eq!(redacted.len(), 4);
    assert!(redacted
        .iter()
        .any(|(k, v)| k == "Authorization" && v == "***REDACTED***"));
    assert!(redacted
        .iter()
        .any(|(k, v)| k == "X-API-Key" && v == "***REDACTED***"));
    assert!(redacted
        .iter()
        .any(|(k, v)| k == "X-Secret-Token" && v == "***REDACTED***"));
    assert!(redacted
        .iter()
        .any(|(k, v)| k == "Content-Type" && v == "application/json"));
}

#[test]
fn redact_headers_does_not_alter_original() {
    let headers = vec![("Authorization".into(), "Bearer real-token".into())];
    let _ = HttpRequestTool::redact_headers_for_display(&headers);
    assert_eq!(headers[0].1, "Bearer real-token");
}

#[test]
fn redirect_policy_is_none() {
    let tool = test_tool(vec!["example.com"]);
    assert_eq!(tool.name(), "http_request");
}
