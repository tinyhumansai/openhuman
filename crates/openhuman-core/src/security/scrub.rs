//! Fail-closed synchronous adapters to TinySecurity's shared redactor.

use serde_json::Value;

/// Summary of whether the module rewrote the value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SanitizationReport {
    /// Whether TinySecurity changed or withheld content.
    pub changed: bool,
}

impl SanitizationReport {
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }
}

/// Sanitized value paired with a content-free report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized<T> {
    pub value: T,
    pub report: SanitizationReport,
}

const REDACTED: &str = "[REDACTED]";
const MAX_SYNC_BYTES: usize = 64 * 1024;

/// Redact free text through TinySecurity, withholding it if the module cannot answer.
#[must_use]
pub fn sanitize_text(value: &str) -> Sanitized<String> {
    let result = if value.len() > MAX_SYNC_BYTES {
        None
    } else {
        redact(value, tinysecurity_bus::RedactMode::Text)
    };
    let cleaned = result.unwrap_or_else(|| REDACTED.to_owned());
    Sanitized {
        report: SanitizationReport {
            changed: cleaned != value,
        },
        value: cleaned,
    }
}

/// Redact structured arguments through TinySecurity, replacing the complete
/// value with a shape-only marker if the bus cannot answer or its result is invalid.
#[must_use]
pub fn sanitize_json(value: &Value) -> Sanitized<Value> {
    let serialized = serde_json::to_string(value).ok();
    let result = serialized.as_deref().and_then(|text| {
        if text.len() > MAX_SYNC_BYTES {
            None
        } else {
            redact(text, tinysecurity_bus::RedactMode::StructuredArguments)
        }
    });
    let cleaned = result
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| Value::String(unavailable_shape(value)));
    Sanitized {
        report: SanitizationReport {
            changed: serialized
                .as_deref()
                .is_none_or(|before| cleaned.to_string().as_str() != before),
        },
        value: cleaned,
    }
}

fn unavailable_shape(value: &Value) -> String {
    let shape = match value {
        Value::Object(map) => format!("object ({} keys)", map.len()),
        Value::Array(items) => format!("array ({} items)", items.len()),
        Value::String(text) => format!("string ({} chars)", text.chars().count()),
        Value::Null => return "<redacted: null>".to_owned(),
        _ => "scalar".to_owned(),
    };
    format!("<redacted: {shape}; arguments unavailable>")
}

fn redact(text: &str, mode: tinysecurity_bus::RedactMode) -> Option<String> {
    #[cfg(feature = "security-module")]
    {
        crate::modules::security::text::redact_blocking(
            mode,
            text,
            std::time::Duration::from_secs(2),
        )
        .ok()
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = (text, mode);
        None
    }
}

#[cfg(test)]
#[path = "scrub_tests.rs"]
mod tests;
