//! Composio-backed Slack provider.
//!
//! Drives Slack history ingestion **without** a user-managed bot token
//! — authorization lives in the user's Composio Slack connection, and
//! the actual API calls fan out through [`ComposioClient::execute_tool`]
//! against Composio's action catalog (`SLACK_LIST_CONVERSATIONS`,
//! `SLACK_FETCH_CONVERSATION_HISTORY`, `SLACK_FETCH_TEAM_INFO`, …).
//!
//! The product provider retains profile lookup, trigger handling, and response
//! normalization. Channel enumeration, cursors, history paging, and memory
//! ingestion execute in tinycortex's Slack sync pipeline.
//!
//! ## Idempotency
//!
//! Source id is `slack:{connection_id}` — stable per workspace. Chunk
//! IDs are stable, so repeated synchronization updates the same documents.

use crate::openhuman::memory_sync::composio::providers::{
    pick_str, resolve_sync_interval_secs, ComposioProvider, CuratedTool, ProviderContext,
    ProviderUserProfile, SyncOutcome,
};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Composio action slug for team/workspace profile fetch.
const ACTION_FETCH_TEAM_INFO: &str = "SLACK_FETCH_TEAM_INFO";
/// Composio action slug for Slack `auth.test` — returns the authed
/// user's id, handle, and team. Required for self-identity capture.
const ACTION_AUTH_TEST: &str = "SLACK_TEST_AUTH";
/// Composio action slug for Slack `users.info` — returns the user's
/// profile (email, real_name, avatar). Optional; needs `users:read.email`
/// scope for the email field.
const ACTION_USERS_INFO: &str = "SLACK_RETRIEVE_DETAILED_USER_INFORMATION";

/// Default backfill window (days) applied when a channel has no
/// cursor yet.
pub const BACKFILL_DAYS: i64 = 6;

/// Sync cadence for provider catalog scheduling.
const SYNC_INTERVAL_SECS: u64 = 15 * 60;

pub struct SlackProvider;

impl SlackProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SlackProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for SlackProvider {
    fn toolkit_slug(&self) -> &'static str {
        "slack"
    }

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(crate::openhuman::memory_sync::composio::providers::catalogs::SLACK_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        Some(resolve_sync_interval_secs("slack", SYNC_INTERVAL_SECS))
    }

    fn post_process_action_result(
        &self,
        slug: &str,
        arguments: Option<&serde_json::Value>,
        data: &mut serde_json::Value,
    ) {
        super::post_process::post_process(slug, arguments, data);
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:slack] fetch_user_profile via {ACTION_AUTH_TEST}"
        );

        // Step 1 — auth.test: required. Returns user_id (canonical sender
        // id on Slack messages), the user's handle, and the team.
        let auth_resp = ctx
            .execute(ACTION_AUTH_TEST, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:slack] {ACTION_AUTH_TEST} failed: {e:#}"))?;

        if !auth_resp.successful {
            let err = auth_resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:slack] {ACTION_AUTH_TEST}: {err}"));
        }

        // `auth_data` is the inner Composio payload — paths are relative
        // to it. Slack's auth.test returns user_id/user/team/team_id at
        // the top of `data`.
        let auth_data = &auth_resp.data;
        let user_id = pick_str(auth_data, &["user_id"]);
        let handle = pick_str(auth_data, &["user"]);
        let team_id = pick_str(auth_data, &["team_id"]);
        let team_name = pick_str(auth_data, &["team"]);

        // Step 2 — users.info: optional. Needs `users:read.email` scope
        // for `email`; falls back to `auth.test` data on missing-scope or
        // any other failure so the profile still carries user_id+handle.
        let mut display_name: Option<String> = None;
        let mut email: Option<String> = None;
        let mut avatar_url: Option<String> = None;

        if let Some(uid) = user_id.as_deref() {
            match ctx
                .execute(ACTION_USERS_INFO, Some(json!({ "user": uid })))
                .await
            {
                Ok(info) if info.successful => {
                    let d = &info.data;
                    email = pick_str(d, &["user.profile.email", "profile.email"]);
                    display_name = pick_str(
                        d,
                        &[
                            "user.profile.real_name",
                            "user.real_name",
                            "user.profile.display_name",
                        ],
                    );
                    avatar_url = pick_str(d, &["user.profile.image_192", "user.profile.image_72"]);
                }
                Ok(info) => {
                    tracing::info!(
                        connection_id = ?ctx.connection_id,
                        error = ?info.error,
                        "[composio:slack] {ACTION_USERS_INFO} returned non-success — \
                         falling back to auth.test data only (likely missing users:read scope)"
                    );
                }
                Err(e) => {
                    tracing::info!(
                        connection_id = ?ctx.connection_id,
                        error = %e,
                        "[composio:slack] {ACTION_USERS_INFO} call failed — \
                         falling back to auth.test data only"
                    );
                }
            }
        }

        // Step 3 — team_info: optional. Adds workspace context to `extras`
        // (email_domain, icon) so the prompt section / UI can show it.
        let (team_email_domain, team_icon) =
            match ctx.execute(ACTION_FETCH_TEAM_INFO, Some(json!({}))).await {
                Ok(resp) if resp.successful => {
                    let d = &resp.data;
                    let domain = pick_str(d, &["team.email_domain", "email_domain"]);
                    let icon = pick_str(d, &["team.icon.image_132", "team.icon.image_68"]);
                    (domain, icon)
                }
                _ => (None, None),
            };

        // Display name preference: users.info real_name > auth.test handle
        // > team_name (last-resort so the prompt isn't empty).
        let final_display_name = display_name
            .clone()
            .or_else(|| handle.clone())
            .or_else(|| team_name.clone());

        // Profile URL: users.info doesn't return one for the user
        // directly; the workspace URL is acceptable as a navigational
        // fallback. (Slack user profile pages are workspace-scoped and
        // not stably linkable from auth.test alone.)
        let profile_url = pick_str(auth_data, &["url"]);

        let avatar_url = avatar_url.or(team_icon);

        let profile = ProviderUserProfile {
            toolkit: "slack".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name: final_display_name,
            email,
            // username carries the platform-canonical sender id so the
            // self-identity matcher can compare against Slack message
            // sender_user_id directly. Handle moves into `extras` —
            // `expand_identity_rows` lifts it back out as IdentityKind::Handle.
            username: user_id,
            avatar_url,
            profile_url,
            extras: json!({
                "handle": handle,
                "team_id": team_id,
                "team_name": team_name,
                "team_email_domain": team_email_domain,
            }),
        };

        let has_email = profile.email.is_some();
        let email_domain = profile
            .email
            .as_deref()
            .and_then(|e| e.split('@').nth(1))
            .map(|d| d.to_string());
        tracing::info!(
            connection_id = ?profile.connection_id,
            has_email,
            email_domain = ?email_domain,
            has_user_id = profile.username.is_some(),
            "[composio:slack] fetched user profile"
        );
        Ok(profile)
    }

    /// Slack rides the generic orchestrator. Channel enumeration + the user
    /// directory backfill happen in [`super::source::SlackSource::preamble`];
    /// per-channel `conversations.history` pagination, the per-channel `oldest`
    /// watermark, dedup, the `max_items` cap, and per-channel error tolerance
    /// all live in `run_sync`. The Slack-specific primitives live in
    /// [`super::source`].
    async fn on_trigger(
        &self,
        ctx: &ProviderContext,
        trigger: &str,
        _payload: &Value,
    ) -> Result<(), String> {
        if trigger.to_ascii_uppercase().contains("MESSAGE") {
            let Some(connection_id) = ctx.connection_id.as_deref() else {
                return Err("[composio:slack] trigger missing connection_id".to_string());
            };
            if let Err(e) = crate::openhuman::tinycortex::run_composio_connection(
                "slack",
                connection_id,
                ctx.config.as_ref(),
            )
            .await
            {
                tracing::warn!(
                    error = %e,
                    "[composio:slack] trigger-driven sync failed (non-fatal)"
                );
            }
        }
        Ok(())
    }
}

// ── Search-based backfill (one-shot) ────────────────────────────────

/// Compatibility wrapper for the tinycortex workspace-wide search pipeline.
pub async fn run_backfill_via_search(
    ctx: &ProviderContext,
    backfill_days: i64,
) -> Result<SyncOutcome, String> {
    let connection_id = ctx
        .connection_id
        .as_deref()
        .ok_or_else(|| "[composio:slack] search backfill missing connection_id".to_string())?;
    let started_at_ms = now_ms();
    let outcome = crate::openhuman::tinycortex::run_slack_search_backfill(
        connection_id,
        backfill_days,
        ctx.config.as_ref(),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(SyncOutcome {
        toolkit: "slack".into(),
        connection_id: Some(connection_id.into()),
        reason: "manual".into(),
        items_ingested: outcome.records_ingested as usize,
        started_at_ms,
        finished_at_ms: now_ms(),
        summary: outcome
            .note
            .unwrap_or_else(|| "slack search-backfill complete".into()),
        details: serde_json::json!({
            "more_pending": outcome.more_pending,
            "actions_called": outcome.actions_called,
            "provider_cost_usd": outcome.provider_cost_usd,
        }),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolkit_slug_is_stable() {
        assert_eq!(SlackProvider::new().toolkit_slug(), "slack");
    }

    #[test]
    fn sync_interval_matches_constant() {
        assert_eq!(
            SlackProvider::new().sync_interval_secs(),
            Some(SYNC_INTERVAL_SECS)
        );
    }

    #[test]
    fn curated_tools_returns_slack_catalog() {
        let tools = SlackProvider::new().curated_tools().unwrap();
        assert!(tools
            .iter()
            .any(|t| t.slug == "SLACK_FETCH_CONVERSATION_HISTORY"));
        assert!(tools.iter().any(|t| t.slug == "SLACK_LIST_CONVERSATIONS"));
    }

    #[test]
    fn post_process_action_result_delegates_to_post_process_module() {
        let provider = SlackProvider::new();
        let mut data = serde_json::json!({
            "channels": [{"id": "C1", "name": "eng", "is_private": false}]
        });
        // Calling with an unknown slug should be a no-op.
        provider.post_process_action_result("SLACK_UNKNOWN_ACTION", None, &mut data);
        assert!(
            data.get("channels").is_some(),
            "no-op slug must not mutate data"
        );
    }
}
