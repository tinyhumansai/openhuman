//! `tinyhumans_sdk::Error` → core [`BackendTransportError`], one arm per
//! variant the core classifies on so `backend::client::finish_authed_json` and
//! the integrations client see exactly the shapes they saw when they called
//! the SDK directly.

use super::port::{BackendTransportError, BillingRefusal};
use reqwest::Method;
use serde_json::Value;
use tinyhumans_sdk::Error as SdkError;

/// Extract the provider and message id from a channel-message route, allowing
/// a fixed API prefix such as `/api/v1` before `/channels`.
fn channel_message_path(path: &str) -> Option<(&str, &str)> {
    let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let index = segments.iter().position(|segment| *segment == "channels")?;
    match segments.get(index..)? {
        ["channels", provider, "messages", message_id]
            if !provider.is_empty() && !message_id.is_empty() =>
        {
            Some((*provider, *message_id))
        }
        _ => None,
    }
}

/// The backend's unmatched Express route responds with plain text or HTML;
/// handler errors are JSON and mean the message itself was not found.
fn is_unmatched_route_404(body: &serde_json::Value) -> bool {
    matches!(body, serde_json::Value::String(_))
}

/// The backend `errorCode` for an account whose balance is below what the
/// route reserves (e.g. Gemini Live's per-session reserve).
const INSUFFICIENT_CREDITS_ERROR_CODE: &str = "USER_INSUFFICIENT_CREDITS";

/// Classify a non-2xx response as a billing refusal: an
/// `errorCode: USER_INSUFFICIENT_CREDITS` envelope on any status but 401
/// (TAURI-RUST-13BR, whose message matches no budget phrase; a 401 stays a
/// credential failure), or a 403 whose `error` text asks to add credits or
/// names a paid plan (the voice-agent signed-url gate, TAURI-RUST-11G3, which
/// answers with `errorCode: FORBIDDEN`).
/// Only the `error` field is read for 403s, so a body that merely echoes a
/// phrase elsewhere does not match.
pub(crate) fn billing_refusal(status: u16, body: &Value) -> Option<BillingRefusal> {
    if status != 401
        && body.get("errorCode").and_then(Value::as_str) == Some(INSUFFICIENT_CREDITS_ERROR_CODE)
    {
        return Some(BillingRefusal::InsufficientCredits);
    }
    if status != 403 {
        return None;
    }
    let error = body.get("error").and_then(Value::as_str)?;
    if error.to_ascii_lowercase().contains("paid plan") {
        Some(BillingRefusal::PaidPlanRequired)
    } else if tinyinference_providers::is_budget_exhausted_message(error) {
        Some(BillingRefusal::InsufficientCredits)
    } else {
        None
    }
}

/// Map the SDK's error for `method path` onto the core-owned transport error.
///
/// A `404` on a channel-message route becomes a typed variant here, because
/// only this backend's wire behaviour says what it means: a handler's JSON
/// 404 is a message that no longer exists, while an unmatched-route 404
/// (Express `finalhandler` HTML) on `PATCH` is the edit route the backend never
/// implemented (#5230) — the message is still there. The core recovers from
/// the two differently and never inspects the body itself.
pub fn map_sdk_error(error: SdkError, method: &Method, path: &str) -> BackendTransportError {
    if let SdkError::Status { status: 404, .. } = &error {
        if let Some((provider, message_id)) = channel_message_path(path) {
            let (provider, message_id) = (provider.to_owned(), message_id.to_owned());
            if *method == Method::PATCH
                && matches!(&error, SdkError::Status { body, .. } if is_unmatched_route_404(body))
            {
                log::debug!(
                    "[tinyhumans-transport] {method} {path}: 404 with no matching route; \
                     channel edit route missing"
                );
                return BackendTransportError::ChannelMessageRouteMissing {
                    provider,
                    message_id,
                };
            }
            log::debug!("[tinyhumans-transport] {method} {path}: channel message not found");
            return BackendTransportError::ChannelMessageNotFound {
                provider,
                message_id,
            };
        }
    }
    match error {
        SdkError::Url(e) => BackendTransportError::Url(e.to_string()),
        SdkError::Http(e) => BackendTransportError::Http(e),
        SdkError::Status { status, body } => match billing_refusal(status, &body) {
            Some(refusal) => {
                log::debug!("[tinyhumans-transport] {method} {path}: billing refusal {refusal:?}");
                BackendTransportError::BillingRefusal {
                    refusal,
                    status,
                    body,
                }
            }
            None => BackendTransportError::Status { status, body },
        },
        SdkError::Header(e) => BackendTransportError::Header(e.to_string()),
        SdkError::Decode(e) => BackendTransportError::Decode(e.to_string()),
        SdkError::RouteNotExposed(method, path) => {
            BackendTransportError::RouteNotExposed(method, path)
        }
        SdkError::Envelope {
            error,
            error_code,
            details,
        } => BackendTransportError::Envelope {
            error,
            error_code,
            details,
        },
        // Socket.IO variants only exist with the SDK's `socket` feature, which
        // this crate turns off; kept as a catch-all so a future SDK variant
        // maps to something rather than failing the build.
        #[allow(unreachable_patterns)]
        other => BackendTransportError::Other(other.to_string()),
    }
}
