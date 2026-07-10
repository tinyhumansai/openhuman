//! Announcements RPC ops — a thin adapter that calls the hosted API.
//!
//! # Security
//! Requires a valid app-session JWT stored via `auth_store_session` (same guard
//! as `billing/ops.rs`). The JWT is sent as `Authorization: Bearer …`; the
//! backend decides what the user may see. No authorization is replicated here.
//! A lapsed session surfaces the backend 401 verbatim via `flatten_authed_error`.

use reqwest::Method;
use serde_json::Value;

use crate::api::config::effective_backend_api_url;
use crate::api::{BackendApiError, BackendOAuthClient};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Canonical authed-session guard — rejects an expired token locally instead of
/// firing a doomed backend 401 (see `billing/ops.rs` / #3297).
fn require_token(config: &Config) -> Result<String, String> {
    crate::openhuman::credentials::session_support::require_live_session_token(config)
}

/// `true` when `err` is the typed `BackendApiError::AnnouncementNotFound` 404
/// (see `src/api/rest.rs`) — the backend has no announcement for this user,
/// which is a normal outcome for this best-effort feature, not a failure.
fn is_announcement_not_found(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<BackendApiError>(),
        Some(BackendApiError::AnnouncementNotFound)
    )
}

/// Fetch the latest active announcement for the signed-in user.
/// Maps to `GET /announcements/latest`. The backend returns the announcement
/// object or `null` when nothing qualifies; both pass through verbatim.
///
/// A 404 (`BackendApiError::AnnouncementNotFound`) is folded into that same
/// "no announcement" contract instead of propagating as an error — this
/// feature is best-effort/cosmetic, and surfacing the 404 as a hard failure
/// flooded Sentry with no actionable signal (TAURI-RUST-HW0, TAURI-RUST-KHX).
/// Any other error (5xx, malformed response, session expiry, …) still
/// propagates via `flatten_authed_error` and still reaches Sentry.
pub async fn get_latest_announcement(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let token = require_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;

    match client
        .authed_json(&token, Method::GET, "/announcements/latest", None)
        .await
    {
        Ok(data) => Ok(RpcOutcome::single_log(data, "latest announcement fetched")),
        Err(err) if is_announcement_not_found(&err) => Ok(RpcOutcome::single_log(
            Value::Null,
            "no announcement available (404)",
        )),
        Err(err) => Err(crate::api::flatten_authed_error(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcement_not_found_error_is_detected() {
        let err = anyhow::Error::new(BackendApiError::AnnouncementNotFound);
        assert!(is_announcement_not_found(&err));
    }

    #[test]
    fn other_backend_errors_are_not_announcement_not_found() {
        let err = anyhow::Error::new(BackendApiError::Unauthorized {
            method: "GET".to_string(),
            path: "/announcements/latest".to_string(),
        });
        assert!(!is_announcement_not_found(&err));

        let plain = anyhow::anyhow!("GET /announcements/latest failed (500): boom");
        assert!(!is_announcement_not_found(&plain));
    }
}
