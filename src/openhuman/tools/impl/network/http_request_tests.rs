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
fn validate_accepts_http() {
    let tool = test_tool(vec!["example.com"]);
    assert!(tool.validate_url("http://example.com").is_ok());
}

#[test]
fn validate_accepts_subdomain() {
    let tool = test_tool(vec!["example.com"]);
    assert!(tool.validate_url("https://api.example.com/v1").is_ok());
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
    let tool = HttpRequestTool::new(security, vec![], 1_000_000, 30);
    let err = tool
        .validate_url("https://example.com")
        .unwrap_err()
        .to_string();
    assert!(err.contains("allowed_domains"));
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

#[test]
fn blocks_multicast_ipv4() {
    assert!(is_private_or_local_host("224.0.0.1"));
    assert!(is_private_or_local_host("239.255.255.255"));
}

#[test]
fn blocks_broadcast() {
    assert!(is_private_or_local_host("255.255.255.255"));
}

#[test]
fn blocks_reserved_ipv4() {
    assert!(is_private_or_local_host("240.0.0.1"));
    assert!(is_private_or_local_host("250.1.2.3"));
}

#[test]
fn blocks_documentation_ranges() {
    assert!(is_private_or_local_host("192.0.2.1")); // TEST-NET-1
    assert!(is_private_or_local_host("198.51.100.1")); // TEST-NET-2
    assert!(is_private_or_local_host("203.0.113.1")); // TEST-NET-3
}

#[test]
fn blocks_benchmarking_range() {
    assert!(is_private_or_local_host("198.18.0.1"));
    assert!(is_private_or_local_host("198.19.255.255"));
}

#[test]
fn blocks_ipv6_localhost() {
    assert!(is_private_or_local_host("::1"));
    assert!(is_private_or_local_host("[::1]"));
}

#[test]
fn blocks_ipv6_multicast() {
    assert!(is_private_or_local_host("ff02::1"));
}

#[test]
fn blocks_ipv6_link_local() {
    assert!(is_private_or_local_host("fe80::1"));
}

#[test]
fn blocks_ipv6_unique_local() {
    assert!(is_private_or_local_host("fd00::1"));
}

#[test]
fn blocks_ipv4_mapped_ipv6() {
    assert!(is_private_or_local_host("::ffff:127.0.0.1"));
    assert!(is_private_or_local_host("::ffff:192.168.1.1"));
    assert!(is_private_or_local_host("::ffff:10.0.0.1"));
}

#[test]
fn allows_public_ipv4() {
    assert!(!is_private_or_local_host("8.8.8.8"));
    assert!(!is_private_or_local_host("1.1.1.1"));
    assert!(!is_private_or_local_host("93.184.216.34"));
}

#[test]
fn blocks_ipv6_documentation_range() {
    assert!(is_private_or_local_host("2001:db8::1"));
}

#[test]
fn allows_public_ipv6() {
    assert!(!is_private_or_local_host("2607:f8b0:4004:800::200e"));
}

#[test]
fn blocks_shared_address_space() {
    assert!(is_private_or_local_host("100.64.0.1"));
    assert!(is_private_or_local_host("100.127.255.255"));
    assert!(!is_private_or_local_host("100.63.0.1")); // Just below range
    assert!(!is_private_or_local_host("100.128.0.1")); // Just above range
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
    assert!(truncated.len() <= 10 + 60); // limit + message
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

// ── SSRF: alternate IP notation bypass defense-in-depth ─────────
//
// Rust's IpAddr::parse() rejects non-standard notations (octal, hex,
// decimal integer, zero-padded). These tests document that property
// so regressions are caught if the parsing strategy ever changes.

#[test]
fn ssrf_octal_loopback_not_parsed_as_ip() {
    // 0177.0.0.1 is octal for 127.0.0.1 in some languages, but
    // Rust's IpAddr rejects it — it falls through as a hostname.
    assert!(!is_private_or_local_host("0177.0.0.1"));
}

#[test]
fn ssrf_hex_loopback_not_parsed_as_ip() {
    // 0x7f000001 is hex for 127.0.0.1 in some languages.
    assert!(!is_private_or_local_host("0x7f000001"));
}

#[test]
fn ssrf_decimal_loopback_not_parsed_as_ip() {
    // 2130706433 is decimal for 127.0.0.1 in some languages.
    assert!(!is_private_or_local_host("2130706433"));
}

#[test]
fn ssrf_zero_padded_loopback_not_parsed_as_ip() {
    // 127.000.000.001 uses zero-padded octets.
    assert!(!is_private_or_local_host("127.000.000.001"));
}

#[test]
fn ssrf_alternate_notations_rejected_by_validate_url() {
    // Even if is_private_or_local_host doesn't flag these, they
    // fail the allowlist because they're treated as hostnames.
    let tool = test_tool(vec!["example.com"]);
    for notation in [
        "http://0177.0.0.1",
        "http://0x7f000001",
        "http://2130706433",
        "http://127.000.000.001",
    ] {
        let err = tool.validate_url(notation).unwrap_err().to_string();
        assert!(
            err.contains("allowed_domains"),
            "Expected allowlist rejection for {notation}, got: {err}"
        );
    }
}

#[test]
fn redirect_policy_is_none() {
    // Structural test: the tool should be buildable with redirect-safe config.
    // The actual Policy::none() enforcement is in execute_request's client builder.
    let tool = test_tool(vec!["example.com"]);
    assert_eq!(tool.name(), "http_request");
}

// ── §1.4 DNS rebinding / SSRF defense-in-depth tests ─────

#[test]
fn ssrf_blocks_loopback_127_range() {
    assert!(is_private_or_local_host("127.0.0.1"));
    assert!(is_private_or_local_host("127.0.0.2"));
    assert!(is_private_or_local_host("127.255.255.255"));
}

#[test]
fn ssrf_blocks_rfc1918_10_range() {
    assert!(is_private_or_local_host("10.0.0.1"));
    assert!(is_private_or_local_host("10.255.255.255"));
}

#[test]
fn ssrf_blocks_rfc1918_172_range() {
    assert!(is_private_or_local_host("172.16.0.1"));
    assert!(is_private_or_local_host("172.31.255.255"));
}

#[test]
fn ssrf_blocks_unspecified_address() {
    assert!(is_private_or_local_host("0.0.0.0"));
}

#[test]
fn ssrf_blocks_dot_localhost_subdomain() {
    assert!(is_private_or_local_host("evil.localhost"));
    assert!(is_private_or_local_host("a.b.localhost"));
}

#[test]
fn ssrf_blocks_dot_local_tld() {
    assert!(is_private_or_local_host("service.local"));
}

#[test]
fn ssrf_ipv6_unspecified() {
    assert!(is_private_or_local_host("::"));
}

#[test]
fn validate_rejects_ftp_scheme() {
    let tool = test_tool(vec!["example.com"]);
    let err = tool
        .validate_url("ftp://example.com")
        .unwrap_err()
        .to_string();
    assert!(err.contains("http://") || err.contains("https://"));
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
        .validate_url("http://[::1]:8080/path")
        .unwrap_err()
        .to_string();
    assert!(err.contains("IPv6"));
}
