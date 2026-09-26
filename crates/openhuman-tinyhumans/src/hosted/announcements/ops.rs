//! Announcements RPC ops — a thin adapter over the SDK's typed
//! `announcements()` client.
//!
//! # Security
//! Authenticates with the core's resolved backend credential
//! ([`HostedClient`]); the backend decides what the user may see. No
//! authorization is replicated here.

use serde_json::Value;
use tinyhumans_sdk::Error as SdkError;

use openhuman_core::config::Config;
use openhuman_core::rpc::RpcOutcome;

use crate::hosted::client::HostedClient;

/// `true` when `result` is the backend's 404 for "no announcement" — a normal
/// outcome for this best-effort feature, not a failure.
fn is_not_found<T>(result: &Result<T, SdkError>) -> bool {
    matches!(result, Err(SdkError::Status { status: 404, .. }))
}

/// Fetch the latest active announcement for the signed-in user.
/// Maps to `GET /announcements/latest`. The backend returns the announcement
/// object or `null` when nothing qualifies; both pass through.
///
/// A 404 is folded into that same "no announcement" contract instead of
/// propagating as an error — this feature is best-effort/cosmetic, and
/// surfacing the 404 as a hard failure flooded Sentry with no actionable
/// signal (TAURI-RUST-HW0, TAURI-RUST-KHX). Any other error (5xx, a response
/// that no longer matches the announcement schema, session expiry, …) still
/// propagates.
pub async fn get_latest_announcement(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let result = client
        .sdk()
        .announcements()
        .get_latest_announcements()
        .await;
    if is_not_found(&result) {
        log::debug!("[hosted][announcements] 404 on GET /announcements/latest — no announcement");
        return Ok(RpcOutcome::single_log(
            Value::Null,
            "no announcement available (404)",
        ));
    }
    let announcement = client.finish("GET /announcements/latest", result)?;
    let data = serde_json::to_value(announcement)
        .map_err(|e| format!("failed to encode announcement: {e}"))?;
    Ok(RpcOutcome::single_log(data, "latest announcement fetched"))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
