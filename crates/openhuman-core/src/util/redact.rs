//! PII redaction for log output.
//!
//! Per `CLAUDE.md`: "Never log secrets or full PII." Source ids, entity ids and
//! content paths can embed full email addresses, so any log line printing one
//! has to redact it first.
//!
//! # Why this is OpenHuman's and not the memory crate's
//!
/// Redact an endpoint for safe logging through TinySecurity. Query parameters
/// and fragments are never retained. If the module is unavailable, suppress it.
#[must_use]
pub fn redact_url_for_log(raw: &str) -> String {
    #[cfg(feature = "security-module")]
    {
        crate::modules::security::text::redact_blocking(
            tinysecurity_bus::RedactMode::Endpoint,
            raw.trim(),
            std::time::Duration::from_millis(250),
        )
        .unwrap_or_else(|_| crate::core::log_redaction::SUPPRESSED.to_owned())
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = raw;
        crate::core::log_redaction::SUPPRESSED.to_owned()
    }
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
