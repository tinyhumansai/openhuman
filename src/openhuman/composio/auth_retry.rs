//! Single-shot retry wrapper around [`ComposioClient::execute_tool`] for
//! the post-OAuth token-propagation gap (issue #1688).
//!
//! Composio reports `connection.status == ACTIVE` ~1-2s after the user
//! finishes OAuth, but its action-execution gateway can take another
//! 30-60s to sync the new token into its execution cache, run scope
//! validation against the upstream provider, and step out of its
//! first-use rate limit. During that window the gateway returns the
//! literal string `"Connection error, try to authenticate"` for normal
//! action calls — the connection is genuinely active and a second call
//! seconds later succeeds.
//!
//! The retry here is intentionally narrow:
//!
//! * single re-attempt only, after a fixed short sleep
//! * gated on a small constant list of well-known auth-error strings so
//!   a real revoked or mis-scoped connection still surfaces to the user
//!   after exactly one round-trip — never an infinite loop
//! * only the payload-level `successful = false / error = "…"` shape is
//!   eligible; transport-level errors (HTTP non-2xx, bad envelope, connect
//!   failures) propagate unchanged because the upstream
//!   [`crate::openhuman::integrations`] client already classifies and
//!   retries those separately.

use std::time::Duration;

use super::client::ComposioClient;
use super::types::ComposioExecuteResponse;

/// Literal error strings Composio's gateway emits during the post-OAuth
/// readiness gap. Matching is `error.contains(needle)` so trailing
/// punctuation and capitalisation drift on the gateway side does not
/// silently disable the retry.
pub(crate) const RETRYABLE_AUTH_ERRORS: &[&str] = &["Connection error, try to authenticate"];

/// Backoff before the single retry. 8s sits in the middle of the 5-10s
/// recommendation in issue #1688 — long enough for Composio's action
/// gateway to sync the token, short enough that a genuine auth failure
/// surfaces to the user well inside the orchestrator's per-turn budget.
pub(crate) const AUTH_RETRY_BACKOFF: Duration = Duration::from_secs(8);

/// Execute `slug` against the Composio gateway and, on a known
/// post-OAuth auth-error payload, retry exactly once after
/// [`AUTH_RETRY_BACKOFF`]. The second response is returned verbatim,
/// even if it is itself an error — callers see exactly what the gateway
/// produced.
pub(crate) async fn execute_with_auth_retry(
    client: &ComposioClient,
    slug: &str,
    args: Option<serde_json::Value>,
) -> anyhow::Result<ComposioExecuteResponse> {
    execute_with_auth_retry_inner(client, slug, args, AUTH_RETRY_BACKOFF).await
}

/// Test-visible inner form that takes an explicit backoff so unit tests
/// can drive the retry path without sleeping for real seconds.
pub(crate) async fn execute_with_auth_retry_inner(
    client: &ComposioClient,
    slug: &str,
    args: Option<serde_json::Value>,
    backoff: Duration,
) -> anyhow::Result<ComposioExecuteResponse> {
    tracing::debug!(
        target: "composio",
        slug = %slug,
        has_args = args.is_some(),
        "[composio][auth_retry] execute start"
    );
    let first = client.execute_tool(slug, args.clone()).await?;
    if first.successful {
        tracing::debug!(
            target: "composio",
            slug = %slug,
            "[composio][auth_retry] first attempt successful; no retry"
        );
        return Ok(first);
    }
    let err_text = first.error.as_deref().unwrap_or("");
    let matched = match_retryable_auth_error(err_text);
    let Some(matched) = matched else {
        // Surface the response unchanged. We deliberately do NOT log
        // `err_text` here — upstream provider messages can embed
        // identifiers (emails, file IDs, channel names) and a `warn`
        // line at every non-retryable failure would broadcast them.
        tracing::debug!(
            target: "composio",
            slug = %slug,
            has_error = !err_text.is_empty(),
            "[composio][auth_retry] non-retryable payload; returning first response"
        );
        return Ok(first);
    };
    tracing::warn!(
        target: "composio",
        slug = %slug,
        retry_reason = matched,
        sleep_ms = backoff.as_millis() as u64,
        "[composio] post-OAuth auth error on first action call; sleeping and retrying once (#1688)"
    );
    tokio::time::sleep(backoff).await;
    let second = client.execute_tool(slug, args).await?;
    tracing::debug!(
        target: "composio",
        slug = %slug,
        successful = second.successful,
        "[composio][auth_retry] retry attempt completed"
    );
    Ok(second)
}

/// Returns the matched needle (a static label safe to log) when the
/// provider error text matches one of [`RETRYABLE_AUTH_ERRORS`]. Match
/// is case-insensitive so capitalisation drift on Composio's side does
/// not silently disable the retry.
fn match_retryable_auth_error(err: &str) -> Option<&'static str> {
    if err.is_empty() {
        return None;
    }
    let err_lc = err.to_ascii_lowercase();
    RETRYABLE_AUTH_ERRORS
        .iter()
        .copied()
        .find(|needle| err_lc.contains(&needle.to_ascii_lowercase()))
}

#[cfg(test)]
fn is_retryable_auth_error(err: &str) -> bool {
    match_retryable_auth_error(err).is_some()
}

#[cfg(test)]
#[path = "auth_retry_tests.rs"]
mod tests;
