use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    })
}

#[test]
fn web_fetch_name_and_schema() {
    let tool = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
    assert_eq!(tool.name(), "web_fetch");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["url"].is_object());
    assert!(schema["required"]
        .as_array()
        .unwrap()
        .contains(&json!("url")));
}

#[test]
fn zero_and_none_limits_fall_back_to_defaults() {
    // Callers wire these from `[http_request]`; a stale `Some(0)` is a
    // 0-byte cap (empty bodies) and a 0-second timeout (instant failure).
    // Both `None` and `Some(0)` must coerce to the shared schema defaults.
    let defaults = crate::openhuman::config::HttpRequestConfig::default();
    let from_zero = WebFetchTool::new(
        test_security(),
        vec!["example.com".into()],
        Some(0),
        Some(0),
    );
    assert_eq!(from_zero.max_bytes, defaults.max_response_size);
    assert_eq!(from_zero.timeout_secs, defaults.timeout_secs);
    assert_ne!(from_zero.timeout_secs, 0);
    assert_ne!(from_zero.max_bytes, 0);

    let from_none = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
    assert_eq!(from_none.max_bytes, defaults.max_response_size);
    assert_eq!(from_none.timeout_secs, defaults.timeout_secs);
}

#[test]
fn nonzero_limits_are_preserved() {
    let tool = WebFetchTool::new(
        test_security(),
        vec!["example.com".into()],
        Some(4096),
        Some(15),
    );
    assert_eq!(tool.max_bytes, 4096);
    assert_eq!(tool.timeout_secs, 15);
}

#[tokio::test]
async fn web_fetch_rejects_disallowed_domain() {
    let tool = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
    let result = tool
        .execute(json!({ "url": "https://evil.test/path" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("URL rejected"));
}

#[tokio::test]
async fn web_fetch_rejects_invalid_url() {
    let tool = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
    let result = tool.execute(json!({ "url": "not-a-url" })).await.unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn web_fetch_blocked_under_local_only_privacy_mode() {
    // Privacy epic S7 (#4441): under LocalOnly the fetch is refused with a
    // `[policy-blocked]` result before any URL validation / network.
    let _mode = super::super::local_only_scope();
    let tool = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
    let result = tool
        .execute(json!({ "url": "https://example.com/data" }))
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
fn test_web_fetch_truncation_utf8() {
    // Mock body with multi-byte char exactly at budget
    let body = "Hello 🦀 World"; // 🦀 is at index 6-9
    let max_bytes = 8;
    // Should truncate at index 6
    let cut = crate::openhuman::util::floor_char_boundary(body, max_bytes);
    assert_eq!(cut, 6);
    assert_eq!(&body[..cut], "Hello ");
}
