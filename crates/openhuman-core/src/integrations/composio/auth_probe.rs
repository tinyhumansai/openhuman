//! Mode-aware Composio sign-in probe used by sub-agent integration refresh.

// ─────────────────────────────────────────────────────────────────────────────
// Composio sign-in probe
// ─────────────────────────────────────────────────────────────────────────────

/// Probe whether the user can call Composio at all under the current
/// config. Returns `true` when the mode-aware factory can build EITHER
/// a backend-mode client (legacy JWT-driven path) OR a direct-mode
/// client (BYO Composio API key). The resolved client is dropped
/// immediately — this is purely a "signed-in vs not" check used by the
/// spawn-time refresh path. Per-action dispatch resolves a fresh client
/// elsewhere via [`crate::integrations::composio::client::resolve_composio_route`] so the live `composio.mode`
/// toggle keeps winning.
///
/// Extracted as a free function so the regression suite can exercise
/// the same probe the runner uses without spinning up the full
/// `run_typed_mode` plumbing.
pub(crate) async fn user_is_signed_in_to_composio(config: &crate::config::Config) -> bool {
    crate::integrations::composio::client::resolve_composio_route(config)
        .await
        .is_ok()
}
