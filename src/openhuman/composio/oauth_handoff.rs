//! OAuth handoff helpers — Meta (Instagram / Facebook) rate-limit mitigations (#1952).
//!
//! Meta's OAuth authorize endpoint returns HTTP 429 when too many OAuth sessions
//! are created in a short window. That often happens when a user retries after a
//! failed handoff or clicks Connect multiple times, leaving several `PENDING`
//! Composio rows that each redirect through Meta. Before starting a new handoff
//! for Meta-owned toolkits we clear prior non-active connection rows and apply
//! a small backoff retry when the backend reports a 429-shaped failure.

use std::time::Duration;

use super::client::{direct_authorize, ComposioClient};
use super::types::ComposioAuthorizeResponse;

/// Toolkits whose OAuth flows are hosted by Meta and share the same rate limits.
pub const META_OAUTH_TOOLKITS: &[&str] = &["instagram", "facebook"];

const AUTHORIZE_RATE_LIMIT_MAX_ATTEMPTS: u32 = 3;
const AUTHORIZE_RATE_LIMIT_INITIAL_BACKOFF: Duration = Duration::from_secs(5);
const AUTHORIZE_RATE_LIMIT_MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Return true when `toolkit` uses Meta-hosted OAuth (Instagram / Facebook).
pub fn is_meta_oauth_toolkit(toolkit: &str) -> bool {
    let key = toolkit.trim().to_ascii_lowercase();
    META_OAUTH_TOOLKITS.contains(&key.as_str())
}

/// Status values that mean an OAuth handoff is still in flight.
pub fn is_inflight_oauth_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_uppercase().as_str(),
        "PENDING" | "INITIATED" | "INITIALIZING"
    )
}

/// Non-active rows safe to delete before starting a fresh Meta OAuth handoff.
pub fn is_clearable_oauth_status(status: &str) -> bool {
    let upper = status.trim().to_ascii_uppercase();
    is_inflight_oauth_status(status) || matches!(upper.as_str(), "FAILED" | "ERROR" | "EXPIRED")
}

/// Detect authorize-path failures that look like upstream rate limiting.
pub fn is_authorize_rate_limited(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("429")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("ratelimited")
}

/// User-facing hint when Meta OAuth is rate-limited.
pub fn meta_oauth_rate_limit_message(toolkit: &str) -> String {
    let name = toolkit.trim();
    let account_hint = if name.eq_ignore_ascii_case("instagram") {
        " Use an Instagram Business or Creator account — personal accounts are not supported."
    } else if name.eq_ignore_ascii_case("facebook") {
        " Confirm the Facebook account has access to the relevant Page or Business Manager."
    } else {
        ""
    };
    format!(
        "Meta is temporarily rate-limiting {name} sign-in (HTTP 429). Wait a few \
         minutes before retrying and avoid clicking Connect repeatedly.{account_hint}"
    )
}

/// If `err` is a Meta-toolkit authorize rate limit, replace it with guidance.
pub fn wrap_authorize_rate_limit_error(toolkit: &str, err: anyhow::Error) -> anyhow::Error {
    let rendered = format!("{err:#}");
    if is_meta_oauth_toolkit(toolkit) && is_authorize_rate_limited(&rendered) {
        anyhow::anyhow!("{}", meta_oauth_rate_limit_message(toolkit))
    } else {
        err
    }
}

/// Remove non-active connection rows for `toolkit` so a fresh OAuth handoff does
/// not accumulate Meta sessions (#1952).
pub async fn clear_non_active_connections(
    client: &ComposioClient,
    toolkit: &str,
) -> anyhow::Result<u32> {
    if !is_meta_oauth_toolkit(toolkit) {
        return Ok(0);
    }
    let toolkit_key = toolkit.trim().to_ascii_lowercase();
    let resp = client.list_connections().await?;
    let mut cleared = 0u32;
    for conn in resp.connections {
        if conn.normalized_toolkit() != toolkit_key {
            continue;
        }
        if conn.is_active() || !is_clearable_oauth_status(&conn.status) {
            continue;
        }
        tracing::info!(
            toolkit = %toolkit_key,
            connection_id = %conn.id,
            status = %conn.status,
            "[composio][oauth] clearing stale non-active connection before Meta OAuth handoff (#1952)"
        );
        match client.delete_connection(&conn.id).await {
            Ok(_) => cleared += 1,
            Err(e) => {
                tracing::warn!(
                    toolkit = %toolkit_key,
                    connection_id = %conn.id,
                    error = %e,
                    "[composio][oauth] failed to clear stale connection (non-fatal)"
                );
            }
        }
    }
    Ok(cleared)
}

/// Begin a backend-proxied OAuth handoff with Meta cleanup + 429 backoff.
pub async fn authorize_with_meta_guard(
    client: &ComposioClient,
    toolkit: &str,
    extra_params: Option<serde_json::Value>,
) -> anyhow::Result<ComposioAuthorizeResponse> {
    let cleared = match clear_non_active_connections(client, toolkit).await {
        Ok(cleared) => cleared,
        Err(e) => {
            tracing::warn!(
                toolkit = %toolkit,
                error = %e,
                "[composio][oauth] pre-handoff cleanup failed; continuing authorize"
            );
            0
        }
    };
    tracing::debug!(
        toolkit = %toolkit,
        cleared,
        is_meta = is_meta_oauth_toolkit(toolkit),
        "[composio][oauth] authorize_with_meta_guard: pre-handoff cleanup"
    );
    authorize_with_rate_limit_retry(|| client.authorize(toolkit, extra_params.clone())).await
}

/// Direct-mode authorize with the same 429 backoff used for Meta toolkits.
pub async fn direct_authorize_with_meta_guard(
    direct: &std::sync::Arc<crate::openhuman::tools::ComposioTool>,
    toolkit: &str,
    entity_id: &str,
) -> anyhow::Result<ComposioAuthorizeResponse> {
    authorize_with_rate_limit_retry(|| direct_authorize(direct, toolkit, entity_id)).await
}

async fn authorize_with_rate_limit_retry<F, Fut>(
    mut attempt_authorize: F,
) -> anyhow::Result<ComposioAuthorizeResponse>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<ComposioAuthorizeResponse>>,
{
    let mut delay = AUTHORIZE_RATE_LIMIT_INITIAL_BACKOFF;
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=AUTHORIZE_RATE_LIMIT_MAX_ATTEMPTS {
        match attempt_authorize().await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                let rendered = format!("{e:#}");
                if is_authorize_rate_limited(&rendered)
                    && attempt < AUTHORIZE_RATE_LIMIT_MAX_ATTEMPTS
                {
                    tracing::warn!(
                        attempt,
                        max_attempts = AUTHORIZE_RATE_LIMIT_MAX_ATTEMPTS,
                        sleep_secs = delay.as_secs(),
                        "[composio][oauth] authorize rate-limited; backing off (#1952)"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(AUTHORIZE_RATE_LIMIT_MAX_BACKOFF);
                    last_err = Some(e);
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("authorize failed after retries")))
}

#[cfg(test)]
#[path = "oauth_handoff_tests.rs"]
mod tests;
