//! Single-shot retry wrapper around [`ComposioClient::execute_tool_once`]
//! for the post-OAuth token-propagation gap (issue #1688).
//!
//! NOTE: PR #1707 later added an in-client retry inside
//! [`ComposioClient::execute_tool`] keyed on the same auth-readiness
//! error string. To avoid stacking two retry layers (which would issue
//! up to four backend calls per logical retry — see the
//! `retries_once_only_even_when_second_call_still_errors` regression),
//! this wrapper calls the non-retrying [`ComposioClient::execute_tool_once`]
//! primitive instead. Direct callers of `execute_tool` (LinkedIn enrichment,
//! heartbeat collectors, tool schemas) still get #1707's inner retry.
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
    let tool = slug.trim();
    if tool.is_empty() {
        tracing::debug!(
            target: "composio",
            raw_slug_len = slug.len(),
            "[composio][auth_retry] rejecting empty tool slug"
        );
        anyhow::bail!("composio.execute_tool: tool slug must not be empty");
    }
    let arguments = args.unwrap_or(serde_json::Value::Object(Default::default()));
    let has_args = arguments.as_object().is_some_and(|a| !a.is_empty());
    let body = serde_json::json!({ "tool": tool, "arguments": arguments });

    tracing::debug!(
        target: "composio",
        slug = %tool,
        has_args,
        "[composio][auth_retry] execute start"
    );
    client
        .execute_tool_with_post_oauth_retry(tool, &body, backoff)
        .await
}

#[cfg(test)]
#[path = "auth_retry_tests.rs"]
mod tests;
