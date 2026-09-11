use super::*;

#[test]
fn delegates_generic_failure_classes_to_tinyagents() {
    assert!(is_non_retryable(&anyhow::anyhow!(
        "HTTP 401 invalid api key"
    )));
    assert!(is_rate_limited(&anyhow::anyhow!(
        "HTTP 429 rate limit exceeded"
    )));
    assert!(is_upstream_unhealthy(&anyhow::anyhow!(
        "HTTP 503 service unavailable"
    )));
    assert_eq!(
        parse_retry_after_ms(&anyhow::anyhow!("Retry-After: 2.5")),
        Some(2_500)
    );
}

#[test]
fn preserves_openhuman_terminal_account_rules() {
    assert!(is_non_retryable(&anyhow::anyhow!(
        "provider returned: you have reached the limit on your monthly requests"
    )));
    assert!(is_non_retryable(&anyhow::anyhow!(
        "SESSION_EXPIRED: sign in again"
    )));
}
