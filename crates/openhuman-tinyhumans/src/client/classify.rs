//! Classification of a failed `authed_json` round-trip: which failures are
//! expected states (typed [`HostedApiError`]s), which are transient noise, and
//! which are reported.

use crate::transport::BackendTransportError;
use anyhow::{Context, Result};
use openhuman_embed::__host::core::observability::{
    contains_transient_transport_phrase, is_transient_http_status_code, report_error,
};
use openhuman_embed::__host::util::truncate_at_byte_boundary;
use reqwest::Method;
use serde_json::Value;

use super::backend_client::BackendClient;
use super::error::HostedApiError;

/// Max bytes of the `body_shape` key-name list echoed into the report.
/// Bounded so a body with pathologically many keys can't bloat the event.
pub(super) const BACKEND_API_BODY_SHAPE_MAX_BYTES: usize = 120;

/// PII-safe classification of a non-2xx response body for telemetry.
///
/// The report message is written to daily logs before any Sentry scrubbing, so
/// the raw body must never be echoed. Only the SHAPE is emitted: for a JSON
/// object, the count of top-level keys plus the sorted subset that look like
/// schema field names; otherwise a coarse label. Key names are
/// response-controlled too, so only conservative ASCII-identifier keys are
/// echoed and the rest are counted as `redacted`.
pub(super) fn backend_api_body_shape(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "empty".to_string();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => {
            let total = map.len();
            let mut safe: Vec<&str> = map
                .keys()
                .map(String::as_str)
                .filter(|k| is_schema_like_key(k))
                .collect();
            safe.sort_unstable();
            let redacted = total - safe.len();
            let keys = truncate_at_byte_boundary(&safe.join(","), BACKEND_API_BODY_SHAPE_MAX_BYTES);
            format!("object(keys={total},safe=[{keys}],redacted={redacted})")
        }
        Ok(Value::Array(_)) => "array".to_string(),
        Ok(_) => "scalar".to_string(),
        Err(_) => "non_json".to_string(),
    }
}

/// A JSON key safe to echo into telemetry: a short ASCII identifier.
fn is_schema_like_key(key: &str) -> bool {
    const MAX_KEY_LEN: usize = 40;
    !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

impl BackendClient {
    /// Turn a transport result into the `authed_json` result, classifying
    /// failures. See [`HostedApiError`] for the typed outcomes.
    pub(super) fn finish_authed_json(
        &self,
        method: Method,
        path: &str,
        response: Result<Value, BackendTransportError>,
        is_api_key: bool,
    ) -> Result<Value> {
        let url = self.url_for(path)?;
        let (status_code, response_body) = match response {
            Ok(value) => return Ok(value),
            Err(BackendTransportError::Unavailable) => {
                return Err(anyhow::Error::new(HostedApiError::BackendUnavailable {
                    method: method.as_str().to_string(),
                    path: url.path().to_string(),
                }));
            }
            Err(BackendTransportError::Http(e)) => {
                // Walk the source chain so transient markers hidden in nested
                // causes (reqwest -> hyper -> rustls TLS EOF) still classify.
                let mut error_message = e.to_string();
                let mut src: Option<&(dyn std::error::Error + 'static)> =
                    std::error::Error::source(&e);
                while let Some(s) = src {
                    error_message.push_str(" → ");
                    error_message.push_str(&s.to_string());
                    src = s.source();
                }
                if contains_transient_transport_phrase(&error_message) {
                    tracing::warn!(
                        domain = "backend_api",
                        operation = "authed_json",
                        method = method.as_str(),
                        path = url.path(),
                        failure = "transport",
                        error = %error_message,
                        "[backend_api] transient transport failure on {} {}: {}",
                        method.as_str(),
                        url.path(),
                        error_message,
                    );
                } else {
                    report_error(
                        error_message.as_str(),
                        "backend_api",
                        "authed_json",
                        &[
                            ("method", method.as_str()),
                            ("path", url.path()),
                            ("failure", "transport"),
                        ],
                    );
                }
                return Err(anyhow::Error::new(e).context(format!(
                    "backend request {} {}",
                    method.as_str(),
                    url.path()
                )));
            }
            // The transport classifies channel-message 404s (it owns the
            // backend's wire behaviour); the recovery each implies is ours.
            Err(BackendTransportError::ChannelMessageRouteMissing {
                provider,
                message_id,
            }) => {
                tracing::warn!(
                    domain = "backend_api",
                    operation = "authed_json",
                    provider = provider,
                    message_id = message_id,
                    "[backend_api] channel-message 404 on {} {}: backend implements no such \
                     route; surfacing ChannelEditUnsupported (#5230)",
                    method.as_str(),
                    url.path(),
                );
                return Err(anyhow::Error::new(HostedApiError::ChannelEditUnsupported {
                    provider,
                    message_id,
                }));
            }
            Err(BackendTransportError::ChannelMessageNotFound {
                provider,
                message_id,
            }) => {
                tracing::info!(
                    domain = "backend_api",
                    operation = "authed_json",
                    provider = provider,
                    message_id = message_id,
                    "[backend_api] message-not-found 404 on {} {}: surfacing typed error",
                    method.as_str(),
                    url.path(),
                );
                return Err(anyhow::Error::new(HostedApiError::MessageNotFound {
                    provider,
                    message_id,
                }));
            }
            Err(BackendTransportError::Status { status, body }) => (status, body),
            Err(error) => {
                return Err(anyhow::Error::new(error).context(format!(
                    "backend request {} {}",
                    method.as_str(),
                    url.path()
                )));
            }
        };

        let status = reqwest::StatusCode::from_u16(status_code)
            .context("backend transport returned an invalid HTTP status")?;
        let text = match response_body {
            Value::String(text) => text,
            other => serde_json::to_string(&other).unwrap_or_default(),
        };

        // 401 on any authed endpoint is an expected session state (token
        // expired / revoked / rotated), not a code bug. The credential kind
        // decides the recovery: a session expires, an API key is rejected.
        if status_code == 401 {
            tracing::info!(
                domain = "backend_api",
                operation = "authed_json",
                method = method.as_str(),
                path = url.path(),
                status = status_code,
                failure = "non_2xx",
                "[backend_api] 401 on {} {}: credential rejected, surfacing typed error",
                method.as_str(),
                url.path(),
            );
            let (method, path) = (method.as_str().to_string(), url.path().to_string());
            return Err(anyhow::Error::new(if is_api_key {
                HostedApiError::ApiKeyRejected { method, path }
            } else {
                HostedApiError::Unauthorized { method, path }
            }));
        }

        // Transient infrastructure errors (proxy/CDN/backend unavailable) and
        // exhausted budgets are not code bugs; callers retry or disable.
        let is_transient_infra = is_transient_http_status_code(status_code);
        let is_budget_exhausted =
            status_code == 400 && tinyinference_providers::is_budget_exhausted_message(&text);
        if is_budget_exhausted {
            tracing::info!(
                method = method.as_str(),
                path = url.path(),
                status = status_code,
                failure = "non_2xx",
                kind = "budget",
                "[backend_api] budget-exhausted 400 on {} {}: not reporting to Sentry",
                method.as_str(),
                url.path(),
            );
        } else if is_transient_infra {
            tracing::warn!(
                domain = "backend_api",
                operation = "authed_json",
                method = method.as_str(),
                path = url.path(),
                status = status_code,
                failure = "non_2xx",
                "[backend_api] transient {status} on {} {}: not reporting to Sentry",
                method.as_str(),
                url.path(),
            );
        } else {
            // Host and JSON key names locate misrouted backend errors without
            // sending response values (TAURI-RUST-8C).
            let host = url.host_str().unwrap_or("");
            let body_shape = backend_api_body_shape(&text);
            let status_str = status_code.to_string();
            report_error(
                format!(
                    "{} {} failed ({status}); response_body_len={}; body_shape={}",
                    method.as_str(),
                    url.path(),
                    text.len(),
                    body_shape,
                )
                .as_str(),
                "backend_api",
                "authed_json",
                &[
                    ("method", method.as_str()),
                    ("path", url.path()),
                    ("host", host),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
        }
        anyhow::bail!(
            "{} {} failed ({status}): {text}",
            method.as_str(),
            url.path()
        )
    }
}
