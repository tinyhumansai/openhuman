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
fn zero_limits_fall_back_to_defaults() {
    // Stale configs (or a bad write) can pass 0; a 0-second timeout fails
    // every request and a 0-byte cap truncates every body. The constructor
    // must coerce both to the module defaults — never let 0 reach reqwest.
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    });
    let tool = HttpRequestTool::new(security, vec!["example.com".to_string()], 0, 0);
    let defaults = crate::openhuman::config::HttpRequestConfig::default();
    assert_eq!(tool.max_response_size, defaults.max_response_size);
    assert_eq!(tool.timeout_secs, defaults.timeout_secs);
    assert_ne!(tool.timeout_secs, 0);
    assert_ne!(tool.max_response_size, 0);
}

#[test]
fn nonzero_limits_are_preserved() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    });
    let tool = HttpRequestTool::new(security, vec!["example.com".to_string()], 2048, 12);
    assert_eq!(tool.max_response_size, 2048);
    assert_eq!(tool.timeout_secs, 12);
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
async fn validate_url_rejects_disallowed_domain() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool
        .validate_url("https://evil.test/path")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("allowed websites"));
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

#[tokio::test]
async fn execute_blocked_under_local_only_privacy_mode() {
    // Privacy epic S7 (#4441): under LocalOnly the request is refused with a
    // `[policy-blocked]` result before URL validation / network.
    let _mode = super::super::local_only_scope();
    let tool = HttpRequestTool::new(
        Arc::new(SecurityPolicy::default()),
        vec!["example.com".into()],
        1_000_000,
        30,
    );
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("[policy-blocked]"),
        "got: {}",
        result.output()
    );
    assert!(
        result.output().contains("Local-only"),
        "got: {}",
        result.output()
    );
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

#[test]
fn supervised_http_request_is_external_effect_for_approval_gate() {
    let tool = test_tool(vec!["example.com"]);
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert!(tool.external_effect_with_args(&json!({
        "url": "https://example.com/api",
        "method": "POST",
        "headers": { "Authorization": "Bearer token" },
        "body": "{}"
    })));
}

#[test]
fn readonly_http_request_is_not_external_effect_because_execute_blocks() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = HttpRequestTool::new(security, vec!["example.com".into()], 1_000_000, 30);
    assert!(!tool.external_effect_with_args(&json!({
        "url": "https://example.com/api",
        "method": "GET"
    })));
}

#[test]
fn egress_descriptor_reports_host_and_url_kind_when_bare() {
    // A bodyless, headerless GET discloses only the destination host + URL.
    let desc = network_egress_descriptor("https://api.example.com/v1/data", false, false);
    assert_eq!(desc.provider_slug, "network");
    assert_eq!(desc.service, "api.example.com");
    assert!(desc.is_external);
    assert_eq!(
        desc.data_kinds,
        vec![crate::openhuman::security::egress::DataKind::Url]
    );
}

#[test]
fn egress_descriptor_adds_tool_arguments_for_body() {
    let desc = network_egress_descriptor("https://api.example.com/v1", true, false);
    assert!(desc
        .data_kinds
        .contains(&crate::openhuman::security::egress::DataKind::ToolArguments));
    assert!(!desc
        .data_kinds
        .contains(&crate::openhuman::security::egress::DataKind::Metadata));
}

#[test]
fn egress_descriptor_reports_headers_as_metadata_even_without_body() {
    // Regression for the under-reporting fix: a header-only call (e.g. an
    // Authorization token, no body) must still disclose the header metadata.
    let desc = network_egress_descriptor("https://api.example.com/v1", false, true);
    assert!(desc
        .data_kinds
        .contains(&crate::openhuman::security::egress::DataKind::Metadata));
}

#[test]
fn egress_descriptor_reports_both_body_and_headers() {
    let desc = network_egress_descriptor("https://api.example.com/v1", true, true);
    let kinds = &desc.data_kinds;
    assert!(kinds.contains(&crate::openhuman::security::egress::DataKind::ToolArguments));
    assert!(kinds.contains(&crate::openhuman::security::egress::DataKind::Metadata));
}

#[test]
fn egress_descriptor_falls_back_to_unknown_host_on_unparseable_url() {
    let desc = network_egress_descriptor("not a url", false, false);
    assert_eq!(desc.service, "unknown");
}
