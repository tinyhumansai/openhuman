
/// String-flat mirror of
/// [`crate::openhuman::inference::provider::error_classify::is_non_retryable_rate_limit`].
///
/// The reliable provider already classifies 429s into retryable vs
/// non-retryable based on business-quota markers ("plan does not
/// include", "insufficient balance", Z.AI codes 1311/1113, …) — but
/// that typed `anyhow::Error` is collapsed to a `String` at the
/// native-bus boundary before reaching this layer. We re-detect the
/// same markers in the flattened string so the FE knows whether to
/// offer a "Retry" button.
///
/// Caller passes the already-lowercased error string to avoid double
/// allocation.
pub(crate) fn is_non_retryable_rate_limit_text(lower: &str) -> bool {
    const BUSINESS_HINTS: &[&str] = &[
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
    if BUSINESS_HINTS.iter().any(|hint| lower.contains(hint)) {
        return true;
    }
    // Known provider business codes observed for 429 where retry is
    // futile (mirrors reliable.rs). Scan integer-like tokens.
    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if matches!(code, 1113 | 1311) {
                return true;
            }
        }
    }
    false
}
