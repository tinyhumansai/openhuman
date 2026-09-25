//! [`redact_composio_outcome`]: keep a Composio API key out of what a
//! Composio tool returns to the model.

use serde_json::Value;
use tinytools::{ToolContent, ToolResult};

use crate::config::Config;

const REDACTED: &str = "[REDACTED]";
const MIN_SECRET_LEN: usize = 8;

/// The Composio keys `config` can resolve to: a host-pinned key, the inline
/// `composio.api_key`, and in unpinned direct mode the stored key.
pub(crate) fn composio_secrets(config: &Config) -> Vec<String> {
    let composio = &config.composio;
    let mut secrets: Vec<String> = Vec::new();
    if let Some(pinned) = composio.host_credential.as_ref() {
        secrets.push(pinned.api_key().to_string());
    }
    if let Some(inline) = composio.api_key.as_deref() {
        secrets.push(inline.trim().to_string());
    }
    if composio.host_credential.is_none()
        && composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_DIRECT
    {
        if let Ok(Some(stored)) = crate::security::credentials::get_composio_api_key(config) {
            secrets.push(stored);
        }
    }
    secrets.retain(|s| s.len() >= MIN_SECRET_LEN);
    secrets.sort();
    secrets.dedup();
    secrets
}

pub(crate) fn redact_text(text: &str, secrets: &[String]) -> String {
    secrets.iter().fold(text.to_string(), |acc, secret| {
        acc.replace(secret.as_str(), REDACTED)
    })
}

fn redact_value(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(s) => {
            if secrets.iter().any(|secret| s.contains(secret.as_str())) {
                *s = redact_text(s, secrets);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| redact_value(v, secrets)),
        Value::Object(map) => map.values_mut().for_each(|v| redact_value(v, secrets)),
        _ => {}
    }
}

fn redact_content(content: &mut ToolContent, secrets: &[String]) {
    match content {
        ToolContent::Text { text } => *text = redact_text(text, secrets),
        ToolContent::Json { data } => redact_value(data, secrets),
        _ => {}
    }
}

pub(crate) fn redact_result(mut result: ToolResult, secrets: &[String]) -> ToolResult {
    if secrets.is_empty() {
        return result;
    }
    result
        .content
        .iter_mut()
        .for_each(|c| redact_content(c, secrets));
    result
        .follow_up
        .iter_mut()
        .for_each(|c| redact_content(c, secrets));
    if let Some(md) = result.markdown_formatted.as_mut() {
        *md = redact_text(md, secrets);
    }
    if let Some(meta) = result.metadata.as_mut() {
        redact_value(meta, secrets);
    }
    result
}

/// Redact every Composio key `config` resolves to from a tool's result or
/// error before it reaches the model.
pub(crate) fn redact_composio_outcome(
    config: &Config,
    outcome: anyhow::Result<ToolResult>,
) -> anyhow::Result<ToolResult> {
    let secrets = composio_secrets(config);
    if secrets.is_empty() {
        return outcome;
    }
    match outcome {
        Ok(result) => Ok(redact_result(result, &secrets)),
        Err(error) => {
            let rendered = format!("{error:#}");
            if secrets.iter().any(|s| rendered.contains(s.as_str())) {
                tracing::debug!("[composio] redacted a credential from a tool error");
                Err(anyhow::anyhow!(redact_text(&rendered, &secrets)))
            } else {
                Err(error)
            }
        }
    }
}
