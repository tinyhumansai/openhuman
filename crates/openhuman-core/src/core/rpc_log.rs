//! Parameter redaction for RPC log lines.
//!
//! [`redact_params_for_log`] is what `core::dispatch` calls for its
//! `[rpc:dispatch] enter` trace line. It delegates structured argument
//! redaction to TinySecurity before any JSON params are logged, since
//! `serde_json::Value` params routinely embed provider credentials.
//!
//! This is the log-side counterpart of `core::log_redaction::scrub_secrets`,
//! which pattern-matches secrets embedded in free-text error strings; this
//! module instead redacts by JSON *key name* in structured params.

use serde_json::Value;

/// Redacts JSON params with TinySecurity's structured-argument policy.
///
/// If the module is unavailable or returns malformed JSON, suppress the whole
/// value rather than risk logging credentials. The module is armed by scoped
/// security policy activation.
fn redact_params_with(params: &Value, redact: impl FnOnce(&str) -> String) -> Value {
    let Ok(encoded) = serde_json::to_string(params) else {
        return Value::String(crate::core::log_redaction::SUPPRESSED.to_owned());
    };
    let redacted = redact(&encoded);
    if redacted == crate::core::log_redaction::SUPPRESSED {
        return Value::String(redacted);
    }
    serde_json::from_str(&redacted)
        .unwrap_or_else(|_| Value::String(crate::core::log_redaction::SUPPRESSED.to_owned()))
}

/// The synchronously bounded shared module redactor used by dispatch logs.
pub fn redact_params_for_log(params: &Value) -> Value {
    #[cfg(feature = "security-module")]
    {
        redact_params_with(params, |text| {
            crate::modules::security::text::redact_for_log(
                crate::modules::security::text::RedactMode::StructuredLogArguments,
                text,
                None,
            )
        })
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = params;
        Value::String(crate::core::log_redaction::SUPPRESSED.to_owned())
    }
}

#[cfg(test)]
#[path = "rpc_log_tests.rs"]
mod tests;
