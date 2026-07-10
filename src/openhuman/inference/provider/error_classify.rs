//! Pure, stateless error-classification / backoff helpers.
//!
//! Extracted from `reliable.rs` (issue #4249, Workstream 02.2) so the
//! transient-vs-permanent classifiers, Retry-After parsing, and failure
//! formatting have a home independent of the `ReliableProvider` retry
//! wrapper. These free functions carry no state — `ReliableProvider` still
//! uses them (via the `pub(crate) use super::error_classify::*;` re-export in
//! `reliable.rs`), and external callers that run their own retry loop over a
//! provider call (`tinyagents::model`, `memory_tree::score::extract::llm`,
//! `agent::triage::evaluator`) classify failures against the same source of
//! truth via the existing `reliable::` paths.

use super::traits::StreamError;
use std::time::Duration;

/// Extract an HTTP `4xx` status code from an error message, but only when it
/// appears in a *structured* position — never from arbitrary digit runs in
/// free text (audit C10). Recognised positions:
///
/// - the documented provider envelope `"… API error (<status>): …"`
///   (e.g. `"OpenAI API error (401 Unauthorized): …"`),
/// - an explicit `HTTP <status>` marker,
/// - a `status: <status>` / `status <status>` field,
/// - a status code that *leads* the message (e.g. `"404 Not Found"`).
///
/// Returns the matched code (always in `400..=499`) or `None`.
pub(crate) fn structured_http_4xx(msg: &str) -> Option<u16> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // (?i) case-insensitive; capture the 4xx in any one of the structured
        // anchors. `\A` matches start-of-string for the leading-status form.
        regex::Regex::new(r"(?i)(?:\(|HTTP\s+|status[:\s]+|\A)(4\d\d)\b")
            .expect("static is_non_retryable 4xx regex is valid")
    });
    re.captures(msg)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u16>().ok())
}

/// Check if an error is non-retryable (client errors that won't resolve with retries).
///
/// `pub(crate)` so other layers that run their own retry loop over a provider
/// call (e.g. `memory_tree::extract::llm`) classify failures against the same
/// source of truth instead of treating every error as a retryable transport
/// blip — retrying a permanent 4xx (402 out of credits, bad key, model gone)
/// only multiplies wasted calls and Sentry events (TAURI-RUST-C62).
pub(crate) fn is_non_retryable(err: &anyhow::Error) -> bool {
    if is_context_window_exceeded(err) {
        return true;
    }
    let msg = err.to_string();
    // Session-expired is a user-auth-state boundary condition, not a
    // transient provider outage. Retrying just burns attempts and delays
    // the sign-in prompt.
    if crate::core::observability::is_session_expired_message(&msg) {
        return true;
    }
    // Monthly-quota / usage-limit exhaustion (e.g. Kiro `MONTHLY_REQUEST_COUNT`,
    // possibly wrapped in a 500 envelope so `structured_http_4xx` can't see the
    // inner 402) is terminal for the period — retrying a spent plan quota only
    // multiplies wasted calls and Sentry events (TAURI-RUST-C9A).
    if crate::openhuman::inference::provider::body_indicates_quota_exhausted(&msg) {
        return true;
    }

    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            let code = status.as_u16();
            return status.is_client_error() && code != 429 && code != 408;
        }
    }
    // Don't infer an HTTP status from *any* free-text digit run — strings like
    // "took 450ms" (→450) or model ids like "gpt-4-0409" (→409) would be
    // misclassified as permanent client errors and short-circuit retries
    // (audit C10). Match only a 4xx code in a *structured* position: the
    // documented `… API error (<status>): …` envelope, an `HTTP <status>` /
    // `status: <status>` marker, or a status that leads the message.
    if let Some(code) = structured_http_4xx(&msg) {
        return code != 429 && code != 408;
    }

    let msg_lower = msg.to_lowercase();
    let auth_failure_hints = [
        "invalid api key",
        "incorrect api key",
        "missing api key",
        "api key not set",
        "authentication failed",
        "auth failed",
        "unauthorized",
        "forbidden",
        "permission denied",
        "access denied",
        "invalid token",
    ];

    if auth_failure_hints
        .iter()
        .any(|hint| msg_lower.contains(hint))
    {
        return true;
    }

    msg_lower.contains("model")
        && (msg_lower.contains("not found")
            || msg_lower.contains("unknown")
            || msg_lower.contains("unsupported")
            || msg_lower.contains("does not exist")
            || msg_lower.contains("invalid"))
}

/// Classify a StreamError without losing type information.
/// Inspects the inner reqwest::Error status directly for Http variants.
pub(crate) fn is_stream_error_non_retryable(err: &StreamError) -> bool {
    match err {
        StreamError::Http(reqwest_err) => {
            if let Some(status) = reqwest_err.status() {
                let code = status.as_u16();
                // Client errors except 429 (rate limit) and 408 (timeout) are non-retryable
                return status.is_client_error() && code != 429 && code != 408;
            }
            false
        }
        StreamError::Provider(msg) => {
            // Mirror the non-streaming classifier: session-expired is a
            // user-auth-state boundary, not a transient provider outage —
            // fail fast so the streaming caller can prompt sign-in instead
            // of burning the retry budget.
            if crate::core::observability::is_session_expired_message(msg) {
                return true;
            }
            let lower = msg.to_lowercase();
            lower.contains("invalid api key")
                || lower.contains("unauthorized")
                || lower.contains("forbidden")
                || lower.contains("model")
                    && (lower.contains("not found") || lower.contains("unsupported"))
        }
        // JSON/SSE parse errors and IO errors are generally non-retryable
        StreamError::Json(_) | StreamError::InvalidSse(_) => true,
        StreamError::Io(_) => false,
    }
}

pub(crate) fn is_context_window_exceeded(err: &anyhow::Error) -> bool {
    // Single source of truth for the context-overflow phrasing lives in
    // `ops::is_context_window_exceeded_message` so the non-retryable
    // classifier here, the `api_error` Sentry-suppression cascade, and the
    // `core::observability` `ContextWindowExceeded` arm can't drift apart.
    super::is_context_window_exceeded_message(&err.to_string())
}

/// Detect provider-side temporary capacity/outage errors. Covers:
///
/// - HTTP `408 Request Timeout`, `502 Bad Gateway`, `503 Service Unavailable`,
///   `504 Gateway Timeout` — both via direct `reqwest::Error` downcast and via
///   the formatted `"<provider> API error (<status>): …"` text emitted by
///   `ops::api_error` (the path that actually reaches `report_error`).
/// - Provider-agnostic text markers like `"no healthy upstream"` /
///   `"upstream unavailable"` that don't come with a typed status.
///
/// Pairs with [`is_rate_limited`] which handles 429 separately. Together they
/// form the transient-classifier the tool-call loop uses before deciding
/// whether to push a per-attempt event to Sentry (see OPENHUMAN-TAURI-2E /
/// -84 / -T / -G classes — per-iteration noise from upstream throttling).
///
/// **Status list maintenance note**: the codes matched below (408/502/503/504)
/// are a subset of
/// [`crate::core::observability::TRANSIENT_PROVIDER_HTTP_STATUSES`] — that
/// const is the single source of truth for the `before_send` filter and the
/// call-site classifier in `providers/ops.rs`. We don't reference the const
/// directly here because this function takes a different code path (anyhow
/// error downcast vs typed `reqwest::StatusCode`) and because 429 is split out
/// into `is_rate_limited` (with its own retry-after parsing). If a new
/// transient status is added to the const, **also add it to this `matches!`
/// arm and the text-pattern list below**.
///
/// Note: 429 lives in `TRANSIENT_PROVIDER_HTTP_STATUSES` but is intentionally
/// absent here — `is_rate_limited` handles it separately because 429 responses
/// may carry a `Retry-After` header that `parse_retry_after_ms` uses to pick a
/// precise backoff rather than the default exponential schedule.
pub(crate) fn is_upstream_unhealthy(err: &anyhow::Error) -> bool {
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            if matches!(status.as_u16(), 408 | 502 | 503 | 504) {
                return true;
            }
        }
    }
    let lower = err.to_string().to_lowercase();
    lower.contains("no healthy upstream")
        || lower.contains("upstream unavailable")
        || lower.contains("service unavailable")
        || lower.contains("503 service unavailable")
        || lower.contains("408 request timeout")
        || lower.contains("502 bad gateway")
        || lower.contains("504 gateway timeout")
}

/// Check if an error is a rate-limit (429) error.
pub(crate) fn is_rate_limited(err: &anyhow::Error) -> bool {
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            return status.as_u16() == 429;
        }
    }
    let msg = err.to_string();
    msg.contains("429")
        && (msg.contains("Too Many") || msg.contains("rate") || msg.contains("limit"))
}

/// Check if a 429 is a business/quota-plan error that retries cannot fix.
///
/// Examples:
/// - plan does not include requested model
/// - insufficient balance / package not active
/// - known provider business codes (e.g. Z.AI: 1311, 1113)
pub(crate) fn is_non_retryable_rate_limit(err: &anyhow::Error) -> bool {
    if !is_rate_limited(err) {
        return false;
    }

    let msg = err.to_string();
    let lower = msg.to_lowercase();

    let business_hints = [
        "plan does not include",
        "doesn't include",
        "not include",
        "insufficient balance",
        "insufficient_balance",
        "insufficient quota",
        "insufficient_quota",
        "quota exhausted",
        "out of credits",
        "no available package",
        "package not active",
        "purchase package",
        "model not available for your plan",
    ];

    if business_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    // Known provider business codes observed for 429 where retry is futile.
    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if matches!(code, 1113 | 1311) {
                return true;
            }
        }
    }

    false
}

/// Try to extract a Retry-After value (in milliseconds) from an error message.
/// Looks for patterns like `Retry-After: 5` or `retry_after: 2.5` in the error string.
pub(crate) fn parse_retry_after_ms(err: &anyhow::Error) -> Option<u64> {
    let msg = err.to_string();
    let lower = msg.to_lowercase();

    // Look for "retry-after: <number>" or "retry_after: <number>"
    for prefix in &[
        "retry-after:",
        "retry_after:",
        "retry-after ",
        "retry_after ",
    ] {
        if let Some(pos) = lower.find(prefix) {
            let after = &msg[pos + prefix.len()..];
            let num_str: String = after
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(secs) = num_str.parse::<f64>() {
                if secs.is_finite() && secs >= 0.0 {
                    let millis = Duration::from_secs_f64(secs).as_millis();
                    if let Ok(value) = u64::try_from(millis) {
                        return Some(value);
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn failure_reason(
    rate_limited: bool,
    non_retryable: bool,
    upstream_unhealthy: bool,
) -> &'static str {
    if upstream_unhealthy {
        "upstream_unhealthy"
    } else if rate_limited && non_retryable {
        "rate_limited_non_retryable"
    } else if rate_limited {
        "rate_limited"
    } else if non_retryable {
        "non_retryable"
    } else {
        "retryable"
    }
}

pub(crate) fn compact_error_detail(err: &anyhow::Error) -> String {
    super::sanitize_api_error(&super::format_anyhow_chain(err))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn rotated_key_log_detail(after_rotate_index: usize, total: usize) -> String {
    let slot = if total == 0 {
        0
    } else {
        after_rotate_index.saturating_sub(1) % total + 1
    };
    format!("slot={slot}/{total}")
}

/// Format the final bail message produced when every provider+model in the
/// chain has failed.
///
/// When the originally-requested `model` has no fallback chain configured
/// in `model_fallbacks`, prepend a single user-actionable hint pointing at
/// the most common cause we see in production (OPENHUMAN-TAURI-BY / -BZ /
/// -C0 / -C1, issue #1596): the user has wired up a `custom_openai`
/// provider whose endpoint does not expose the configured `default_model`.
/// In that scenario the bail aggregate is otherwise an opaque stack of
/// provider-formatted error envelopes which gives the user no clue where
/// to look.
///
/// We deliberately avoid emitting the hint when fallbacks *are* configured
/// — the user has already engaged with the knob and likely has either a
/// real outage or a misconfigured chain; the dump-of-attempts surface is
/// what they need to debug it.
pub(crate) fn format_failure_aggregate(
    model: &str,
    failures: &[String],
    has_configured_fallbacks: bool,
) -> String {
    let attempts = format!(
        "All providers/models failed. Attempts:\n{}",
        failures.join("\n")
    );
    if has_configured_fallbacks {
        attempts
    } else {
        format!(
            "The model `{model}` may not be available on your provider. \
             Configure a fallback chain via `reliability.model_fallbacks` in your \
             OpenHuman config, or change your default model in Connections → API keys → LLM.\n\n{attempts}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_retryable_detects_common_patterns() {
        assert!(is_non_retryable(&anyhow::anyhow!("400 Bad Request")));
        assert!(is_non_retryable(&anyhow::anyhow!("401 Unauthorized")));
        assert!(is_non_retryable(&anyhow::anyhow!("403 Forbidden")));
        assert!(is_non_retryable(&anyhow::anyhow!("404 Not Found")));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "invalid api key provided"
        )));
        assert!(is_non_retryable(&anyhow::anyhow!("authentication failed")));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "model glm-4.7 not found"
        )));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "unsupported model: glm-4.7"
        )));
        assert!(!is_non_retryable(&anyhow::anyhow!("429 Too Many Requests")));
        assert!(!is_non_retryable(&anyhow::anyhow!("408 Request Timeout")));
        assert!(!is_non_retryable(&anyhow::anyhow!(
            "500 Internal Server Error"
        )));
        assert!(!is_non_retryable(&anyhow::anyhow!("502 Bad Gateway")));
        assert!(!is_non_retryable(&anyhow::anyhow!("timeout")));
        assert!(!is_non_retryable(&anyhow::anyhow!("connection reset")));
        assert!(!is_non_retryable(&anyhow::anyhow!(
            "model overloaded, try again later"
        )));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "OpenAI Codex stream error: Your input exceeds the context window of this model."
        )));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "SESSION_EXPIRED: backend session not active — sign in to resume LLM work"
        )));
        // TAURI-RUST-FJZ: the Responses-path error now carries the status in the
        // structured `(<status>)` position, so a terminal 404 from a provider that
        // lacks the Responses API is classified non-retryable and the retry loop
        // stops instead of hammering the permanent 404 (~15k events).
        assert!(is_non_retryable(&anyhow::anyhow!(
            "nous-portal Responses API error (404): Not Found"
        )));
        // The pre-fix form left `404` unanchored (preceded by `error: `), so it
        // slipped past the structured-status regex and looped — guard the regression.
        assert!(
            !is_non_retryable(&anyhow::anyhow!(
                "nous-portal Responses API error: 404 Not Found"
            )),
            "documents the pre-fix misclassification the structured `(404)` form fixes"
        );
    }

    // TAURI-RUST-C9A: a monthly-quota refusal wrapped in a 500 envelope (so the
    // `structured_http_4xx` regex can't see the inner 402) must still be terminal —
    // retrying a spent plan quota only multiplies wasted calls + Sentry events.
    #[test]
    fn non_retryable_detects_monthly_quota_exhaustion() {
        assert!(is_non_retryable(&anyhow::anyhow!(
            "kiro API error (500 Internal Server Error): {{\"error\":{{\"message\":\
             \"HTTP 402 from Kiro IDE: {{\\\"reason\\\":\\\"MONTHLY_REQUEST_COUNT\\\"}}\",\
             \"type\":\"server_error\"}}}}"
        )));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "provider returned: you have reached the limit on your monthly requests"
        )));
        // A generic 500 outage stays retryable (transient) — the quota arm must not
        // over-match.
        assert!(!is_non_retryable(&anyhow::anyhow!(
            "kiro API error (500 Internal Server Error): upstream connection reset"
        )));
    }

    // C10: a 4xx-looking digit run that appears in *free text* (latency figures,
    // model ids, token counts) must NOT be inferred as a permanent HTTP client
    // error — that wrongly short-circuits retries/fallback for transient failures.
    #[test]
    fn non_retryable_ignores_free_text_digit_runs() {
        // "450" here is a latency figure, not a 450 status.
        assert!(
            !is_non_retryable(&anyhow::anyhow!("upstream took 450ms to respond, retrying")),
            "latency figures must not be read as an HTTP status"
        );
        // "0409" embedded in a model id used to scan to 409.
        assert!(
            !is_non_retryable(&anyhow::anyhow!("gpt-4-0409 returned an empty completion")),
            "model-id digits must not be read as an HTTP status"
        );
        // A bare 4xx-shaped token mid-sentence (not in a structured position) is
        // also ignored now.
        assert!(
            !is_non_retryable(&anyhow::anyhow!(
                "received 412 partial bytes before connection reset"
            )),
            "mid-text digit runs must not be read as an HTTP status"
        );
        // Sanity: the structured envelope is still classified as non-retryable.
        assert!(
            is_non_retryable(&anyhow::anyhow!(
                "custom_openai API error (403 Forbidden): nope"
            )),
            "the documented (<status>) envelope must still be detected"
        );
        // Sanity: a leading status (no envelope) is still detected.
        assert!(is_non_retryable(&anyhow::anyhow!("404 Not Found")));
    }

    #[test]
    fn parse_retry_after_integer() {
        let err = anyhow::anyhow!("429 Too Many Requests, Retry-After: 5");
        assert_eq!(parse_retry_after_ms(&err), Some(5000));
    }

    #[test]
    fn parse_retry_after_float() {
        let err = anyhow::anyhow!("Rate limited. retry_after: 2.5 seconds");
        assert_eq!(parse_retry_after_ms(&err), Some(2500));
    }

    #[test]
    fn parse_retry_after_missing() {
        let err = anyhow::anyhow!("500 Internal Server Error");
        assert_eq!(parse_retry_after_ms(&err), None);
    }

    #[test]
    fn rate_limited_detection() {
        assert!(is_rate_limited(&anyhow::anyhow!("429 Too Many Requests")));
        assert!(is_rate_limited(&anyhow::anyhow!(
            "HTTP 429 rate limit exceeded"
        )));
        assert!(!is_rate_limited(&anyhow::anyhow!("401 Unauthorized")));
        assert!(!is_rate_limited(&anyhow::anyhow!(
            "500 Internal Server Error"
        )));
    }

    #[test]
    fn non_retryable_rate_limit_detects_plan_restricted_model() {
        let err = anyhow::anyhow!(
            "{}",
            "API error (429 Too Many Requests): {\"code\":1311,\"message\":\"the current account plan does not include glm-5\"}"
        );
        assert!(
            is_non_retryable_rate_limit(&err),
            "plan-restricted 429 should skip retries"
        );
    }

    #[test]
    fn non_retryable_rate_limit_detects_insufficient_balance() {
        let err = anyhow::anyhow!(
            "{}",
            "API error (429 Too Many Requests): {\"code\":1113,\"message\":\"insufficient balance\"}"
        );
        assert!(
            is_non_retryable_rate_limit(&err),
            "insufficient-balance 429 should skip retries"
        );
    }

    #[test]
    fn non_retryable_rate_limit_does_not_flag_generic_429() {
        let err = anyhow::anyhow!("429 Too Many Requests: rate limit exceeded");
        assert!(
            !is_non_retryable_rate_limit(&err),
            "generic rate-limit 429 should remain retryable"
        );
    }

    // ── §2.1 API auth error (401/403) tests ──────────────────

    #[test]
    fn non_retryable_detects_401() {
        let err = anyhow::anyhow!("API error (401 Unauthorized): invalid api key");
        assert!(
            is_non_retryable(&err),
            "401 errors must be detected as non-retryable"
        );
    }

    #[test]
    fn non_retryable_detects_403() {
        let err = anyhow::anyhow!("API error (403 Forbidden): access denied");
        assert!(
            is_non_retryable(&err),
            "403 errors must be detected as non-retryable"
        );
    }

    #[test]
    fn non_retryable_detects_404() {
        let err = anyhow::anyhow!("API error (404 Not Found): model not found");
        assert!(
            is_non_retryable(&err),
            "404 errors must be detected as non-retryable"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_429() {
        let err = anyhow::anyhow!("429 Too Many Requests");
        assert!(
            !is_non_retryable(&err),
            "429 must NOT be treated as non-retryable (it is retryable with backoff)"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_408() {
        let err = anyhow::anyhow!("408 Request Timeout");
        assert!(
            !is_non_retryable(&err),
            "408 must NOT be treated as non-retryable (it is retryable)"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_500() {
        let err = anyhow::anyhow!("500 Internal Server Error");
        assert!(
            !is_non_retryable(&err),
            "500 must NOT be treated as non-retryable (server errors are retryable)"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_502() {
        let err = anyhow::anyhow!("502 Bad Gateway");
        assert!(
            !is_non_retryable(&err),
            "502 must NOT be treated as non-retryable"
        );
    }

    // ── §2.2 Rate limit Retry-After edge cases ───────────────

    #[test]
    fn parse_retry_after_zero() {
        let err = anyhow::anyhow!("429 Too Many Requests, Retry-After: 0");
        assert_eq!(
            parse_retry_after_ms(&err),
            Some(0),
            "Retry-After: 0 should parse as 0ms"
        );
    }

    #[test]
    fn parse_retry_after_with_underscore_separator() {
        let err = anyhow::anyhow!("rate limited, retry_after: 10");
        assert_eq!(
            parse_retry_after_ms(&err),
            Some(10_000),
            "retry_after with underscore must be parsed"
        );
    }

    #[test]
    fn parse_retry_after_space_separator() {
        let err = anyhow::anyhow!("Retry-After 7");
        assert_eq!(
            parse_retry_after_ms(&err),
            Some(7000),
            "Retry-After with space separator must be parsed"
        );
    }

    #[test]
    fn rate_limited_false_for_generic_error() {
        let err = anyhow::anyhow!("Connection refused");
        assert!(
            !is_rate_limited(&err),
            "generic errors must not be flagged as rate-limited"
        );
    }

    // ── upstream_unhealthy classification and failure_reason precedence ──

    #[test]
    fn upstream_unhealthy_detects_no_healthy_upstream() {
        let err = anyhow::anyhow!("no healthy upstream available");
        assert!(is_upstream_unhealthy(&err));
    }

    #[test]
    fn upstream_unhealthy_detects_upstream_unavailable() {
        let err = anyhow::anyhow!("upstream unavailable: backend down");
        assert!(is_upstream_unhealthy(&err));
    }

    #[test]
    fn upstream_unhealthy_detects_service_unavailable() {
        let err = anyhow::anyhow!("503 service unavailable");
        assert!(is_upstream_unhealthy(&err));
    }

    #[test]
    fn upstream_unhealthy_does_not_flag_generic_error() {
        let err = anyhow::anyhow!("timeout after 30s");
        assert!(!is_upstream_unhealthy(&err));
    }

    // 408/502/504 must also classify as transient — `ops::api_error` formats
    // the upstream failure as "<provider> API error (<status>): <body>", and the
    // tool-call loop ORs is_rate_limited (429) with is_upstream_unhealthy. Before
    // this fix only 503/text-pattern matched; 408/502/504 leaked per-iteration
    // Sentry events (CodeRabbit review on #1529, OPENHUMAN-TAURI-T/-2E/-84).
    #[test]
    fn upstream_unhealthy_detects_408_request_timeout() {
        let err = anyhow::anyhow!("OpenAI API error (408 Request Timeout): upstream took too long");
        assert!(is_upstream_unhealthy(&err));
    }

    #[test]
    fn upstream_unhealthy_detects_502_bad_gateway() {
        let err = anyhow::anyhow!("Anthropic API error (502 Bad Gateway): bad gateway");
        assert!(is_upstream_unhealthy(&err));
    }

    #[test]
    fn upstream_unhealthy_detects_504_gateway_timeout() {
        let err = anyhow::anyhow!("OpenAI API error (504 Gateway Timeout): upstream timed out");
        assert!(is_upstream_unhealthy(&err));
    }

    #[test]
    fn upstream_unhealthy_detects_503_service_unavailable_with_provider_prefix() {
        let err = anyhow::anyhow!("OpenAI API error (503 Service Unavailable): backend overloaded");
        assert!(is_upstream_unhealthy(&err));
    }

    #[test]
    fn failure_reason_upstream_unhealthy_wins_over_rate_limited() {
        // Both rate_limited AND upstream_unhealthy — upstream_unhealthy must win.
        assert_eq!(failure_reason(true, false, true), "upstream_unhealthy");
    }

    #[test]
    fn failure_reason_upstream_unhealthy_wins_over_non_retryable() {
        // Both non_retryable AND upstream_unhealthy — upstream_unhealthy must win.
        assert_eq!(failure_reason(false, true, true), "upstream_unhealthy");
    }

    #[test]
    fn failure_reason_upstream_unhealthy_wins_over_all_others() {
        // All flags set — upstream_unhealthy must still win.
        assert_eq!(failure_reason(true, true, true), "upstream_unhealthy");
    }

    // ── issue #1596: custom_openai model-not-found UX ──

    #[test]
    fn rotated_key_log_detail_does_not_expose_key_suffix() {
        let detail = rotated_key_log_detail(2, 4);

        assert_eq!(detail, "slot=2/4");
        assert!(!detail.contains("sk-"));
        assert!(!detail.contains("..."));
    }

    #[test]
    fn format_failure_aggregate_prepends_user_hint_when_no_fallbacks_configured() {
        let failures = vec![
            "provider=custom_openai model=reasoning-v1 attempt 1/1: non_retryable; \
             error=custom_openai API error (404 Not Found): {\"error\":{\"message\":\
             \"model 'reasoning-v1' not found\"}}"
                .to_string(),
        ];

        let msg = format_failure_aggregate("reasoning-v1", &failures, false);

        assert!(
            msg.contains("may not be available on your provider"),
            "hint copy missing: {msg}"
        );
        assert!(
            msg.contains("reliability.model_fallbacks"),
            "config key reference missing: {msg}"
        );
        assert!(
            msg.contains("Connections → API keys → LLM"),
            "settings pointer missing: {msg}"
        );
        assert!(
            msg.contains("reasoning-v1"),
            "should mention the offending model name: {msg}"
        );
        // The raw attempt dump must still be present for support to diagnose.
        assert!(
            msg.contains("custom_openai API error (404 Not Found)"),
            "raw failure attempts dropped: {msg}"
        );
    }

    #[test]
    fn format_failure_aggregate_omits_hint_when_fallbacks_configured() {
        // User already engaged with `reliability.model_fallbacks`; the
        // configured chain itself failed too. Telling them to "configure a
        // fallback chain" would be misleading — keep the raw dump only.
        let failures = vec![
            "provider=primary model=reasoning-v1 attempt 1/1: non_retryable; error=...".to_string(),
            "provider=primary model=fallback-a attempt 1/1: non_retryable; error=...".to_string(),
        ];

        let msg = format_failure_aggregate("reasoning-v1", &failures, true);

        assert!(
            !msg.contains("Configure a fallback chain"),
            "hint must NOT fire when fallbacks already configured: {msg}"
        );
        assert!(
            msg.starts_with("All providers/models failed."),
            "should use the plain aggregate when user has engaged with the knob: {msg}"
        );
    }
}
