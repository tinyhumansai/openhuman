use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_tool(allowed_domains: Vec<&str>) -> BrowserOpenTool {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    });
    BrowserOpenTool::new(
        security,
        allowed_domains.into_iter().map(String::from).collect(),
    )
}

#[test]
fn normalize_domain_strips_scheme_path_and_case() {
    let got = normalize_domain("  HTTPS://Docs.Example.com/path ").unwrap();
    assert_eq!(got, "docs.example.com");
}

#[test]
fn normalize_allowed_domains_deduplicates() {
    let got = normalize_allowed_domains(vec![
        "example.com".into(),
        "EXAMPLE.COM".into(),
        "https://example.com/".into(),
    ]);
    assert_eq!(got, vec!["example.com".to_string()]);
}

#[test]
fn validate_accepts_exact_domain() {
    let tool = test_tool(vec!["example.com"]);
    let got = tool.validate_url("https://example.com/docs").unwrap();
    assert_eq!(got, "https://example.com/docs");
}

#[test]
fn validate_accepts_subdomain() {
    let tool = test_tool(vec!["example.com"]);
    assert!(tool.validate_url("https://api.example.com/v1").is_ok());
}

#[test]
fn validate_rejects_http() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool
        .validate_url("http://example.com")
        .unwrap_err()
        .to_string();
    assert!(err.contains("https://"));
}

#[test]
fn validate_rejects_localhost() {
    let tool = test_tool(vec!["localhost"]);
    let err = tool
        .validate_url("https://localhost:8080")
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"));
}

#[test]
fn validate_rejects_private_ipv4() {
    let tool = test_tool(vec!["192.168.1.5"]);
    let err = tool
        .validate_url("https://192.168.1.5")
        .unwrap_err()
        .to_string();
    assert!(err.contains("local/private"));
}

#[test]
fn validate_rejects_allowlist_miss() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool
        .validate_url("https://google.com")
        .unwrap_err()
        .to_string();
    assert!(err.contains("allowed_domains"));
}

#[test]
fn validate_rejects_whitespace() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool
        .validate_url("https://example.com/hello world")
        .unwrap_err()
        .to_string();
    assert!(err.contains("whitespace"));
}

#[test]
fn validate_rejects_userinfo() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool
        .validate_url("https://user@example.com")
        .unwrap_err()
        .to_string();
    assert!(err.contains("userinfo"));
}

#[test]
fn validate_requires_allowlist() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserOpenTool::new(security, vec![]);
    let err = tool
        .validate_url("https://example.com")
        .unwrap_err()
        .to_string();
    assert!(err.contains("allowed_domains"));
}

#[test]
fn parse_ipv4_valid() {
    assert_eq!(parse_ipv4("1.2.3.4"), Some([1, 2, 3, 4]));
}

#[test]
fn parse_ipv4_invalid() {
    assert_eq!(parse_ipv4("1.2.3"), None);
    assert_eq!(parse_ipv4("1.2.3.999"), None);
    assert_eq!(parse_ipv4("not-an-ip"), None);
}

#[tokio::test]
async fn execute_blocks_readonly_mode() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = BrowserOpenTool::new(security, vec!["example.com".into()]);
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
}

#[test]
fn validate_rejects_empty_url() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool.validate_url("").unwrap_err().to_string();
    assert!(err.contains("empty"));
}

#[test]
fn validate_rejects_ipv6_host() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool
        .validate_url("https://[::1]:8080/path")
        .unwrap_err()
        .to_string();
    // Rejected as IPv6 (starts with '[')
    assert!(
        err.contains("IPv6") || err.contains("local/private") || err.contains("allowed_domains"),
        "unexpected error: {err}"
    );
}

#[test]
fn is_private_or_local_host_detects_local_tld() {
    assert!(is_private_or_local_host("myhost.local"));
}

#[test]
fn is_private_or_local_host_detects_subdomain_localhost() {
    assert!(is_private_or_local_host("sub.localhost"));
}

#[test]
fn is_private_or_local_host_detects_loopback_ipv6() {
    assert!(is_private_or_local_host("::1"));
}

#[test]
fn is_private_or_local_host_detects_10_range() {
    assert!(is_private_or_local_host("10.0.0.1"));
}

#[test]
fn is_private_or_local_host_detects_0_prefix() {
    assert!(is_private_or_local_host("0.0.0.0"));
}

#[test]
fn is_private_or_local_host_detects_link_local() {
    assert!(is_private_or_local_host("169.254.1.1"));
}

#[test]
fn is_private_or_local_host_detects_cgnat() {
    assert!(is_private_or_local_host("100.64.0.1"));
}

#[test]
fn is_private_or_local_host_allows_public() {
    assert!(!is_private_or_local_host("8.8.8.8"));
    assert!(!is_private_or_local_host("example.com"));
}

#[test]
fn host_matches_allowlist_exact() {
    let domains = vec!["example.com".to_string()];
    assert!(host_matches_allowlist("example.com", &domains));
    assert!(!host_matches_allowlist("other.com", &domains));
}

#[test]
fn host_matches_allowlist_subdomain() {
    let domains = vec!["example.com".to_string()];
    assert!(host_matches_allowlist("sub.example.com", &domains));
    assert!(!host_matches_allowlist("notexample.com", &domains));
}

#[test]
fn normalize_domain_strips_port() {
    assert_eq!(
        normalize_domain("example.com:8080"),
        Some("example.com".into())
    );
}

#[test]
fn normalize_domain_strips_leading_trailing_dots() {
    assert_eq!(
        normalize_domain(".example.com."),
        Some("example.com".into())
    );
}

#[test]
fn normalize_domain_returns_none_for_empty() {
    assert_eq!(normalize_domain(""), None);
    assert_eq!(normalize_domain("   "), None);
}

#[test]
fn normalize_domain_strips_http_prefix() {
    assert_eq!(
        normalize_domain("http://example.com/path"),
        Some("example.com".into())
    );
}

#[test]
fn extract_host_rejects_empty_host() {
    assert!(extract_host("https://").is_err());
}

#[test]
fn extract_host_strips_port() {
    assert_eq!(
        extract_host("https://example.com:443/path").unwrap(),
        "example.com"
    );
}

#[test]
fn extract_host_lowercases() {
    assert_eq!(extract_host("https://EXAMPLE.COM").unwrap(), "example.com");
}

#[test]
fn extract_host_strips_trailing_dot() {
    assert_eq!(
        extract_host("https://example.com./path").unwrap(),
        "example.com"
    );
}

#[test]
fn tool_name_and_description() {
    let tool = test_tool(vec!["example.com"]);
    assert_eq!(tool.name(), "browser_open");
    assert!(!tool.description().is_empty());
}

#[test]
fn parameters_schema_requires_url() {
    let tool = test_tool(vec!["example.com"]);
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("url")));
}

#[tokio::test]
async fn execute_rejects_missing_url_param() {
    let tool = test_tool(vec!["example.com"]);
    let result = tool.execute(json!({})).await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn execute_blocks_when_rate_limited() {
    let security = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let tool = BrowserOpenTool::new(security, vec!["example.com".into()]);
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("rate limit"));
}
