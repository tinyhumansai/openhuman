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
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Canonical authed-session guard — rejects an expired token locally instead of
/// firing a doomed backend 401 (see `billing/ops.rs` / #3297).
fn require_token(config: &Config) -> Result<String, String> {
    crate::openhuman::credentials::session_support::require_live_session_token(config)
}

async fn get_authed_value(config: &Config, method: Method, path: &str) -> Result<Value, String> {
    let token = require_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    client
        .authed_json(&token, method, path, None)
        .await
        .map_err(crate::api::flatten_authed_error)
}

/// Fetch the latest active announcement for the signed-in user.
/// Maps to `GET /announcements/latest`. The backend returns the announcement
/// object or `null` when nothing qualifies; both pass through verbatim.
pub async fn get_latest_announcement(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/announcements/latest").await?;
    Ok(RpcOutcome::single_log(data, "latest announcement fetched"))
}
