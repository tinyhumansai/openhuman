//! Fail-closed synchronous adapters to TinySecurity's shared redactor.
//!
//! With security disabled (`crate::security::switch`) there is no redactor:
//! values pass through unchanged and the report says nothing changed.

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
    if crate::security::switch::disabled().is_some() {
        return unchanged(value.to_owned());
    }
    let result = if value.len() > MAX_SYNC_BYTES {
        None
    } else {
        redact_free_text(value)
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
    if crate::security::switch::disabled().is_some() {
        return unchanged(value.clone());
    }
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

/// A value nobody redacted, because security is disabled.
fn unchanged<T>(value: T) -> Sanitized<T> {
    Sanitized {
        value,
        report: SanitizationReport { changed: false },
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

/// Text-mode redaction of free text that may open with a bracketed label.
///
/// TinySecurity's Text mode parses input that opens with `{`, `[` or `"` as
/// JSON and refuses it when it is not, so host refusals (`[policy-blocked] …`)
/// and harness artifact pointers (`[artifact] kind=…`) would be withheld whole.
/// A leading `[label]` (an ASCII letter, then letters, digits, `-` or `_`, then
/// `]`) cannot open a JSON document, so only that opening bracket is held back:
/// everything after it, the label included, is still redacted by the module,
/// and any failure still withholds the text. Valid JSON and every other input
/// go to the module unchanged.
fn redact_free_text(value: &str) -> Option<String> {
    let body = value.trim_start();
    let lead = &value[..value.len() - body.len()];
    let labelled = body.strip_prefix('[').filter(|rest| {
        let label_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        rest.starts_with(|c: char| c.is_ascii_alphabetic()) && rest[label_len..].starts_with(']')
    });
    match labelled {
        Some(rest) if serde_json::from_str::<serde::de::IgnoredAny>(value).is_err() => {
            log::trace!("[security][scrub] redacting bracket-labelled free text");
            redact(&format!("{lead}{rest}"), tinysecurity_bus::RedactMode::Text).map(|cleaned| {
                let cleaned_body = cleaned.trim_start();
                let cleaned_lead = &cleaned[..cleaned.len() - cleaned_body.len()];
                format!("{cleaned_lead}[{cleaned_body}")
            })
        }
        _ => redact(value, tinysecurity_bus::RedactMode::Text),
    }
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
