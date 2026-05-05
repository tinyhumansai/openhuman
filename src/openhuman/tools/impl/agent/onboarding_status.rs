//! Shared helpers for the welcome agent's onboarding tools.
//!
//! Both `check_onboarding_status` (read-only snapshot) and
//! `complete_onboarding` (finalizer) need the same primitives:
//!
//! * A process-global counter of welcome-agent exchanges this session.
//! * An auth detector (`detect_auth`) that bools out whether a session
//!   JWT is present.
//! * The engagement-criteria gate that decides whether `complete` may
//!   run — at least one app connected (webview login **or** Composio
//!   integration).
//! * The JSON snapshot builder the agent consumes — exposing the list
//!   of connected Composio toolkits and the per-provider webview-login
//!   heuristic (see `openhuman::webview_accounts`).
//!
//! Keeping this in one place lets the two tools stay small and share
//! the same snapshot shape without pulling in tool code from elsewhere.

use crate::openhuman::config::Config;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};

/// Historical exchange-count threshold. No longer used in the
/// engagement gate (which now requires at least one app connected).
/// Retained only for reference; will be removed in a future cleanup.
#[allow(dead_code)]
pub(crate) const MIN_EXCHANGES_TO_COMPLETE: u32 = 3;

/// Process-global exchange counter for the welcome agent.
///
/// Incremented by [`increment_welcome_exchange_count`] (called from the
/// channel dispatch layer) once per inbound user message that routes to
/// the welcome agent. Read by the status tool and by the complete
/// finalizer. Process-local (not persisted) because the welcome flow
/// runs exactly once per fresh install; after completion the counter is
/// never consulted again.
static WELCOME_EXCHANGE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Increment the welcome-agent exchange counter by one.
///
/// Only write site. Called from the channel dispatch layer every time a
/// user message is routed to the welcome agent (i.e. when
/// `chat_onboarding_completed` is `false`).
pub fn increment_welcome_exchange_count() {
    let prev = WELCOME_EXCHANGE_COUNT.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        exchange_count = prev + 1,
        "[onboarding] welcome exchange count incremented"
    );
}

/// Return the current welcome-agent exchange count (process-global).
pub fn get_welcome_exchange_count() -> u32 {
    WELCOME_EXCHANGE_COUNT.load(Ordering::Relaxed)
}

/// Pure-logic helper: returns whether the engagement criteria for
/// `complete_onboarding` are satisfied. The gate is "at least one app
/// connected" — either a webview login (built-in browser app) or a
/// Composio OAuth integration.
pub(crate) fn engagement_criteria_met(
    webview_logins: &serde_json::Value,
    composio_connections: u32,
) -> bool {
    let has_webview = webview_logins
        .as_object()
        .map(|o| o.values().any(|v| v.as_bool().unwrap_or(false)))
        .unwrap_or(false);
    has_webview || composio_connections > 0
}

/// Build the user-facing error string for premature `complete_onboarding`
/// calls. The reason string comes from `compute_state().ready_to_complete_reason`.
pub(crate) fn build_not_ready_to_complete_error(reason: &str) -> String {
    match reason {
        "unauthenticated" => "Cannot complete onboarding: the user is not signed in. \
             Guide them to log in via the desktop login flow first."
            .to_string(),
        "already_complete" => "Onboarding is already complete.".to_string(),
        _ => "Cannot complete onboarding yet: the user hasn't connected any apps. \
             At least one built-in app login (webview) or one connected integration \
             is required before onboarding can be finalized."
            .to_string(),
    }
}

/// Reset the welcome exchange counter to zero. Test-only.
#[cfg(test)]
pub fn reset_welcome_exchange_count() {
    WELCOME_EXCHANGE_COUNT.store(0, Ordering::Relaxed);
}

/// Detect whether the user is authenticated for the welcome flow.
///
/// Authentication is based on the `app-session:default` profile in
/// `auth-profiles.json`, populated by the desktop OAuth deep-link flow.
///
/// Returned as `(is_authenticated, auth_source_json)` so callers can
/// both gate behaviour on the bool and embed the source label in a
/// JSON payload without rebuilding the logic.
pub(crate) fn detect_auth(config: &Config) -> (bool, Value) {
    let has_session_jwt = crate::api::jwt::get_session_token(config)
        .ok()
        .flatten()
        .is_some_and(|t| !t.is_empty());
    let is_authenticated = has_session_jwt;
    let auth_source: Value = if has_session_jwt {
        Value::String("session_token".to_string())
    } else {
        Value::Null
    };
    (is_authenticated, auth_source)
}

/// Build the structured JSON snapshot that the welcome agent consumes.
///
/// Shared between the `check_onboarding_status` tool (reactive) and the
/// proactive welcome path (fired on `onboarding_completed` false→true).
///
/// Beyond the workspace flags, the snapshot carries three signals the
/// agent uses to decide what to offer next:
///
/// * `composio_connected_toolkits` — list of Composio toolkit slugs the
///   user has authorized (e.g. `["gmail", "github"]`). Derived from the
///   same backend call that drives `ready_to_complete`, exposed here so
///   the agent doesn't re-pitch gmail after it's already connected.
/// * `webview_logins` — per-provider bools (gmail, whatsapp, telegram,
///   slack, discord, linkedin, zoom, google_messages) indicating
///   whether the shared CEF cookie store has an active session cookie
///   for that provider. See `openhuman::webview_accounts`.
/// * `exchange_count` / `ready_to_complete` / `ready_to_complete_reason`
///   — the gate the finalizer enforces.
// Channel detection now lives in
// `crate::openhuman::channels::controllers::ops::connected_channel_slugs`
// (precomputed in `compute_state` because it needs to read the
// credential store), so the welcome-agent surface honors managed-DM /
// OAuth connections that don't materialise a TOML
// `channels_config.<slug>` block (issue #1149).

pub(crate) fn build_status_snapshot(
    config: &Config,
    onboarding_status: &str,
    exchange_count: u32,
    ready_to_complete: bool,
    ready_to_complete_reason: &str,
    composio_connected_toolkits: &[String],
    connected_channels: &[String],
    webview_logins: Value,
) -> Value {
    let (is_authenticated, auth_source) = detect_auth(config);
    let channels_connected: Vec<&str> = connected_channels.iter().map(|s| s.as_str()).collect();

    let composio_enabled = config.composio.enabled;
    let delegate_agents: Vec<&str> = config.agents.keys().map(|s| s.as_str()).collect();

    json!({
        "authenticated": is_authenticated,
        "auth_source": auth_source,
        "default_model": config
            .default_model
            .as_deref()
            .unwrap_or(crate::openhuman::config::DEFAULT_MODEL),
        "channels_connected": channels_connected,
        "active_channel": config
            .channels_config
            .active_channel
            .as_deref()
            .unwrap_or("web"),
        "integrations": {
            "composio": composio_enabled,
            "browser": config.browser.enabled,
            "web_search": true,
            "http_request": true,
            "local_ai": config.local_ai.enabled,
        },
        "composio_connected_toolkits": composio_connected_toolkits,
        "webview_logins": webview_logins,
        "memory": {
            "backend": config.memory.backend,
            "auto_save": config.memory.auto_save,
        },
        "delegate_agents": delegate_agents,
        "ui_onboarding_completed": config.onboarding_completed,
        "chat_onboarding_completed": config.chat_onboarding_completed,
        "exchange_count": exchange_count,
        "ready_to_complete": ready_to_complete,
        "ready_to_complete_reason": ready_to_complete_reason,
        "onboarding_status": onboarding_status,
    })
}

/// Render the same onboarding state as `build_status_snapshot` but as
/// compact markdown rather than pretty-printed JSON. Costs ~5x fewer
/// tokens and reads more naturally to the welcome agent. Only fields
/// the welcome flow actually uses (per the agent's prompt.md) are
/// surfaced; everything else (default_model, integrations bools,
/// memory backend, delegate_agents) is dropped.
pub(crate) fn format_status_markdown(
    config: &Config,
    onboarding_status: &str,
    exchange_count: u32,
    ready_to_complete: bool,
    ready_to_complete_reason: &str,
    composio_connected_toolkits: &[String],
    connected_channels: &[String],
    webview_logins: &Value,
) -> String {
    let (is_authenticated, auth_source) = detect_auth(config);
    let channels: Vec<&str> = connected_channels.iter().map(|s| s.as_str()).collect();

    let active_channel = config
        .channels_config
        .active_channel
        .as_deref()
        .unwrap_or("web");

    // Only list `true` webview logins — false ones are noise the agent
    // would have to skip past every turn.
    let webview_active: Vec<String> = webview_logins
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| {
                    if v.as_bool().unwrap_or(false) {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let mut out = String::with_capacity(256);
    out.push_str("# Onboarding Status\n\n");
    out.push_str(&format!(
        "- **status:** {onboarding_status} (ready_to_complete: {ready_to_complete}, reason: {ready_to_complete_reason})\n"
    ));
    out.push_str(&format!(
        "- **auth:** {} ({})\n",
        if is_authenticated { "yes" } else { "no" },
        auth_source.as_str().unwrap_or("none"),
    ));
    out.push_str(&format!("- **exchanges:** {exchange_count}\n"));
    if !composio_connected_toolkits.is_empty() {
        out.push_str(&format!(
            "- **composio:** {}\n",
            composio_connected_toolkits.join(", ")
        ));
    }
    if !webview_active.is_empty() {
        out.push_str(&format!(
            "- **webview logins:** {}\n",
            webview_active.join(", ")
        ));
    }
    if !channels.is_empty() {
        out.push_str(&format!(
            "- **channels:** {} (active: {active_channel})\n",
            channels.join(", ")
        ));
    }
    out.push_str(&format!(
        "- **flags:** ui_onboarding={}, chat_onboarding={}\n",
        config.onboarding_completed, config.chat_onboarding_completed
    ));
    out
}

/// Summarise the current onboarding state for snapshot + finalizer.
///
/// Both tools need the same derived view, so we compute it once here:
/// authenticated? already complete? how many exchanges so far, how many
/// Composio connections, which toolkits, and the resulting
/// `ready_to_complete` gate + reason. Shared code path = shared bugs,
/// so both tools agree on who's ready.
pub(crate) struct OnboardingState {
    pub is_authenticated: bool,
    pub exchange_count: u32,
    pub composio_connected_toolkits: Vec<String>,
    /// Slugs of messaging channels currently connected, merged across
    /// the legacy `channels_config.<slug>` TOML store and the
    /// `channel:<slug>:<mode>` credential store. Computed in
    /// [`compute_state`] (issue #1149).
    pub connected_channels: Vec<String>,
    pub onboarding_status: &'static str,
    pub ready_to_complete: bool,
    pub ready_to_complete_reason: String,
}

pub(crate) async fn compute_state(
    config: &Config,
    webview_logins: &serde_json::Value,
) -> OnboardingState {
    let (is_authenticated, _) = detect_auth(config);
    let exchange_count = get_welcome_exchange_count();
    let integrations = crate::openhuman::composio::fetch_connected_integrations(config).await;
    let composio_connected_toolkits: Vec<String> = integrations
        .iter()
        .filter(|i| i.connected)
        .map(|i| i.toolkit.clone())
        .collect();
    let composio_connections = composio_connected_toolkits.len() as u32;

    // Merge legacy `channels_config.<slug>` with the credential store
    // so managed-DM / OAuth channels (e.g. Telegram managed_dm) report
    // as connected to the welcome agent (issue #1149). Best-effort —
    // a credential-store read failure logs and falls back to empty
    // rather than masking the rest of the snapshot.
    let connected_channels =
        crate::openhuman::channels::controllers::connected_channel_slugs(config)
            .await
            .unwrap_or_else(|err| {
                tracing::warn!(
                    error = %err,
                    "[onboarding] connected_channel_slugs failed; reporting empty channel list"
                );
                Vec::new()
            });

    let onboarding_status = if !is_authenticated {
        "unauthenticated"
    } else if config.chat_onboarding_completed {
        "already_complete"
    } else {
        "pending"
    };

    let ready_to_complete = is_authenticated
        && !config.chat_onboarding_completed
        && engagement_criteria_met(webview_logins, composio_connections);
    let ready_to_complete_reason = if !is_authenticated {
        "unauthenticated".to_string()
    } else if config.chat_onboarding_completed {
        "already_complete".to_string()
    } else if ready_to_complete {
        "criteria_met".to_string()
    } else {
        "no_apps_connected".to_string()
    };

    OnboardingState {
        is_authenticated,
        exchange_count,
        composio_connected_toolkits,
        connected_channels,
        onboarding_status,
        ready_to_complete,
        ready_to_complete_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_status_snapshot_carries_expected_fields() {
        let config = Config::default();
        let snap = build_status_snapshot(
            &config,
            "pending",
            0,
            false,
            "no_apps_connected",
            &[],
            &[],
            json!({"gmail": false}),
        );
        assert_eq!(snap["onboarding_status"], "pending");
        assert_eq!(snap["exchange_count"], 0);
        assert_eq!(snap["ready_to_complete"], false);
        assert_eq!(snap["chat_onboarding_completed"], false);
        assert!(snap["composio_connected_toolkits"].is_array());
        assert_eq!(
            snap["composio_connected_toolkits"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(snap["webview_logins"]["gmail"], false);
        assert!(snap["channels_connected"].is_array());
        assert_eq!(snap["channels_connected"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn build_status_snapshot_carries_connected_toolkits_and_webview() {
        let config = Config::default();
        let snap = build_status_snapshot(
            &config,
            "pending",
            2,
            false,
            "no_apps_connected",
            &["gmail".to_string(), "github".to_string()],
            &[],
            json!({"gmail": true, "whatsapp": false}),
        );
        let toolkits = snap["composio_connected_toolkits"].as_array().unwrap();
        assert_eq!(toolkits[0], "gmail");
        assert_eq!(toolkits[1], "github");
        assert_eq!(snap["webview_logins"]["gmail"], true);
        assert_eq!(snap["webview_logins"]["whatsapp"], false);
    }

    /// Issue #1149: managed-DM / OAuth channels are stored in the
    /// credential layer, not in `channels_config`. The snapshot must
    /// reflect them so the welcome agent doesn't say "Telegram not
    /// connected" right after a managed-DM link succeeds.
    #[test]
    fn build_status_snapshot_surfaces_credential_only_channels() {
        let config = Config::default();
        // `channels_config.telegram` is None — the channel was linked
        // via the managed-DM flow which only writes a credential entry.
        // The merged slug list (built upstream by `compute_state`) is
        // what `build_status_snapshot` consumes.
        let snap = build_status_snapshot(
            &config,
            "pending",
            1,
            false,
            "no_apps_connected",
            &[],
            &["telegram".to_string()],
            json!({}),
        );
        let channels = snap["channels_connected"].as_array().unwrap();
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0], "telegram");
    }

    #[test]
    fn format_status_markdown_surfaces_credential_only_channels() {
        let config = Config::default();
        let md = format_status_markdown(
            &config,
            "pending",
            1,
            false,
            "no_apps_connected",
            &[],
            &["telegram".to_string()],
            &json!({}),
        );
        assert!(
            md.contains("**channels:** telegram"),
            "markdown should surface credential-stored channel: {md}"
        );
    }

    #[test]
    fn detect_auth_on_default_config_is_unauthenticated() {
        let config = Config::default();
        let (is_auth, source) = detect_auth(&config);
        assert!(!is_auth);
        assert!(source.is_null());
    }

    #[test]
    fn exchange_counter_increments_and_resets() {
        reset_welcome_exchange_count();
        assert_eq!(get_welcome_exchange_count(), 0);
        increment_welcome_exchange_count();
        increment_welcome_exchange_count();
        assert_eq!(get_welcome_exchange_count(), 2);
        reset_welcome_exchange_count();
        assert_eq!(get_welcome_exchange_count(), 0);
    }

    #[test]
    fn criteria_not_met_no_webview_no_composio() {
        let logins = json!({"gmail": false, "whatsapp": false});
        assert!(!engagement_criteria_met(&logins, 0));
    }

    #[test]
    fn criteria_not_met_empty_webview() {
        let logins = json!({});
        assert!(!engagement_criteria_met(&logins, 0));
    }

    #[test]
    fn criteria_met_via_webview_login() {
        let logins = json!({"gmail": false, "whatsapp": true});
        assert!(engagement_criteria_met(&logins, 0));
    }

    #[test]
    fn criteria_met_via_composio() {
        let logins = json!({"gmail": false});
        assert!(engagement_criteria_met(&logins, 1));
    }

    #[test]
    fn criteria_met_both_webview_and_composio() {
        let logins = json!({"telegram": true});
        assert!(engagement_criteria_met(&logins, 2));
    }

    #[test]
    fn premature_complete_error_no_apps() {
        let msg = build_not_ready_to_complete_error("no_apps_connected");
        assert!(
            msg.contains("hasn't connected any apps"),
            "unexpected wording: {msg}"
        );
    }

    #[test]
    fn premature_complete_error_unauthenticated() {
        let msg = build_not_ready_to_complete_error("unauthenticated");
        assert!(msg.contains("not signed in"), "unexpected wording: {msg}");
    }

    #[test]
    fn premature_complete_error_already_complete() {
        let msg = build_not_ready_to_complete_error("already_complete");
        assert!(
            msg.contains("already complete"),
            "unexpected wording: {msg}"
        );
    }
}
