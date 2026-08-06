//! OpenHuman-specific policy layered on TinyAgents' provider-neutral retry classifier.

use tinyagents::harness::retry::{
    classify_provider_failure, parse_retry_after_ms as parse_tinyagents_retry_after,
    ProviderFailureClass,
};

fn classify(err: &anyhow::Error) -> ProviderFailureClass {
    let message = err.to_string();

    // Product/account-state failures are terminal even when an upstream proxy
    // wrapped them in a nominally retryable status (for example a 500 carrying
    // MONTHLY_REQUEST_COUNT). These rules intentionally stay host-side.
    if super::is_context_window_exceeded_message(&message)
        || crate::core::observability::is_session_expired_message(&message)
        || crate::openhuman::inference::provider::body_indicates_quota_exhausted(&message)
    {
        return ProviderFailureClass::NonRetryable;
    }

    let status = err
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
        .map(|status| status.as_u16());
    classify_provider_failure(status, None, &message)
}

pub(crate) fn is_non_retryable(err: &anyhow::Error) -> bool {
    matches!(
        classify(err),
        ProviderFailureClass::NonRetryable | ProviderFailureClass::NonRetryableRateLimit
    )
}

pub(crate) fn is_rate_limited(err: &anyhow::Error) -> bool {
    matches!(
        classify(err),
        ProviderFailureClass::RateLimited | ProviderFailureClass::NonRetryableRateLimit
    )
}

pub(crate) fn is_upstream_unhealthy(err: &anyhow::Error) -> bool {
    classify(err) == ProviderFailureClass::UpstreamUnhealthy
}

pub(crate) fn parse_retry_after_ms(err: &anyhow::Error) -> Option<u64> {
    parse_tinyagents_retry_after(&err.to_string())
}

#[cfg(test)]
mod tests {
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
}
