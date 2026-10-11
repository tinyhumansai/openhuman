//! Small helpers shared across the connect/disconnect/status operations.

pub(crate) use tinychannels::controllers::channel_config_connected;

/// Merge a channel's live supervised-listener health into its credential/config
/// derived `connected` flag (issue #3712).
///
/// Only listener-backed modes (those that materialise a TOML config block —
/// `has_config`) have a `channel:<id>` health component, kept current by the
/// supervisor's `ChannelConnected`/`ChannelDisconnected` events. For those, a
/// live `error` overrides the optimistic presence-based `connected` and carries
/// the failure reason to the UI; an `ok` confirms it. While the listener is
/// still `starting` (or has no component yet) we keep the presence-based value
/// so a freshly-configured channel isn't reported as broken before its first
/// connect attempt. Modes without a runtime listener (e.g. managed-DM) are left
/// untouched. Returns `(connected, error)`.
pub(crate) fn merge_listener_health(
    presence_connected: bool,
    has_config: bool,
    health_status: Option<&str>,
    health_last_error: Option<&str>,
) -> (bool, Option<String>) {
    if !has_config {
        return (presence_connected, None);
    }
    match health_status {
        Some("error") => (false, health_last_error.map(str::to_string)),
        Some("ok") => (true, None),
        _ => (presence_connected, None),
    }
}

pub(crate) use crate::security::credentials::channel_credential_provider as credential_provider;
