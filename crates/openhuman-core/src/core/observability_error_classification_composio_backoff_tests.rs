use super::*;

// ── TAURI-RUST-116R: direct-mode invalid-key backoff (gate open) ──

#[test]
fn demotes_composio_direct_key_backoff_message() {
    // Drift coupler: the EXACT message `direct_list_connections` returns once
    // the invalid-key gate is open. `report_composio_op_error` receives it
    // bare (no `[composio-direct]` prefix) on every Settings poll, so it must
    // demote on its own, and still demote once a caller wraps it.
    let bare = crate::integrations::composio::direct_auth::invalid_api_key_backoff_message(3);
    assert_eq!(
        expected_error_kind(&bare),
        Some(ExpectedErrorKind::ProviderUserState),
        "bare backoff message must demote to ProviderUserState: {bare}"
    );
    let wrapped = format!("[composio-direct] list_connections: {bare}");
    assert_eq!(
        expected_error_kind(&wrapped),
        Some(ExpectedErrorKind::ProviderUserState)
    );
    let higher = crate::integrations::composio::direct_auth::invalid_api_key_backoff_message(17);
    assert_eq!(
        expected_error_kind(&higher),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn composio_backoff_anchor_is_prefix_of_backoff_message() {
    use crate::integrations::composio::direct_auth::{
        invalid_api_key_backoff_message, COMPOSIO_DIRECT_KEY_BACKOFF_ANCHOR,
    };
    assert!(
        invalid_api_key_backoff_message(3)
            .to_lowercase()
            .starts_with(COMPOSIO_DIRECT_KEY_BACKOFF_ANCHOR),
        "the 116R anchor must be a lowercase prefix of the backoff message"
    );
}

#[test]
fn composio_backoff_arm_does_not_swallow_unrelated_composio_errors() {
    // Discrimination: a non-auth direct-mode failure (outage) and a
    // backend-mode 401 must not be claimed by the 116R arm.
    let outage = "Composio v3 connected_accounts failed: HTTP 503: upstream unavailable";
    assert_ne!(
        expected_error_kind(outage),
        Some(ExpectedErrorKind::ProviderUserState)
    );
    let unrelated = "Direct-mode cache rejected the snapshot";
    assert_eq!(expected_error_kind(unrelated), None);
}
