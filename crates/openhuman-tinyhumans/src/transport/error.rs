//! `tinyhumans_sdk::Error` → core [`BackendTransportError`], one arm per
//! variant the core classifies on so `backend::client::finish_authed_json` and
//! the integrations client see exactly the shapes they saw when they called
//! the SDK directly.

use super::port::BackendTransportError;
use reqwest::Method;
use tinyhumans_sdk::Error as SdkError;

fn channel_message_path(path: &str) -> Option<(&str, &str)> {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let channels = segments.iter().position(|segment| *segment == "channels")?;
    if segments.get(channels + 2).copied() != Some("messages") {
        return None;
    }
    let provider = *segments.get(channels + 1)?;
    let message_id = *segments.get(channels + 3)?;
    if provider.is_empty() || message_id.is_empty() || segments.len() != channels + 4 {
        return None;
    }
    Some((provider, message_id))
}

fn is_unmatched_route_404(error: &SdkError) -> bool {
    let SdkError::Status { body, .. } = error else {
        return false;
    };
    body.as_str()
        .is_some_and(|body| body.contains("Cannot PATCH "))
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
            if *method == Method::PATCH && is_unmatched_route_404(&error) {
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
        SdkError::Status { status, body } => BackendTransportError::Status { status, body },
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
