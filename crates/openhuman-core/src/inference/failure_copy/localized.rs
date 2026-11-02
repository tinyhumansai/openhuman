//! Rendering of classified native-channel failures in the configured UI locale.

use serde_json::Value;

use super::chat_error_catalog;

const QUEUED_FAILURE_PREFIX: &str = "queued turn was not run: ";

pub(crate) fn supported_channel_locale(locale: Option<&str>) -> Option<&'static str> {
    let locale = locale?.trim();
    chat_error_catalog::SUPPORTED_LOCALES
        .iter()
        .copied()
        .find(|supported| *supported == locale)
}

/// Render known chat-error keys from the shared locale catalog. Unknown keys
/// retain the English message supplied by the host classifier.
pub(crate) fn render_channel_failure(
    locale: &str,
    fallback_message: &str,
    copy_key: Option<&str>,
    copy_params: Option<&Value>,
) -> String {
    let queued = fallback_message
        .strip_prefix(QUEUED_FAILURE_PREFIX)
        .is_some();
    let locale = supported_channel_locale(Some(locale)).unwrap_or("en");
    let Some(key) = copy_key.filter(|key| key.starts_with("chat_error.")) else {
        return fallback_message.to_owned();
    };
    let Some(base) = chat_error_catalog::translation(locale, key) else {
        return fallback_message.to_owned();
    };

    let mut message = base.to_owned();
    if let Some(seconds) = copy_params
        .and_then(|params| params.get("retry_after_secs"))
        .and_then(Value::as_u64)
    {
        let retry_key = match seconds {
            0 => "chat_error.retryHint.immediately",
            1 => "chat_error.retryHint.oneSecond",
            2..=89 => "chat_error.retryHint.seconds",
            _ => {
                let minutes = seconds / 60 + u64::from(seconds % 60 != 0);
                if minutes == 1 {
                    "chat_error.retryHint.aboutMinute"
                } else {
                    "chat_error.retryHint.aboutMinutes"
                }
            }
        };
        if let Some(retry) = chat_error_catalog::translation(locale, retry_key) {
            let rendered = if retry_key.ends_with("seconds") {
                retry.replace("{n}", &seconds.to_string())
            } else if retry_key.ends_with("aboutMinutes") {
                let minutes = seconds / 60 + u64::from(seconds % 60 != 0);
                retry.replace("{n}", &minutes.to_string())
            } else {
                retry.to_owned()
            };
            message.push(' ');
            message.push_str(&rendered);
        }
    }

    if let Some(detail) = copy_params
        .and_then(|params| params.get("detail"))
        .and_then(Value::as_str)
        .filter(|detail| !detail.is_empty())
    {
        message.push_str("\n\n> ");
        message.push_str(detail);
    }
    if queued {
        message.insert_str(0, QUEUED_FAILURE_PREFIX);
    }
    message
}
