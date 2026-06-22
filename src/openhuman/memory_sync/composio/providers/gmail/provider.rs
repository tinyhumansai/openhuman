//! Gmail provider — incremental sync into the memory tree.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Fetch a page of recent messages via `GMAIL_FETCH_EMAILS`, adding
//!      a date filter when a cursor exists so only newer mail is returned.
//!   4. Run [`ComposioProvider::post_process_action_result`] (bounded
//!      HTML→text, normalise, sanitise) on the page so the LLM-facing chunk
//!      content is cleaned, not raw.
//!   5. Filter against `synced_ids` for an early-stop optimisation,
//!      then ingest the new messages into the memory tree via
//!      [`super::ingest::ingest_page_into_memory_tree`] — same pipeline
//!      the standalone `gmail-backfill-3d` binary uses, mirroring the
//!      Slack provider's `ingest_chat` pattern.
//!   6. Paginate (up to budget) until no more results or all items in the
//!      page are already synced.
//!   7. Advance the cursor and save state.
//!
//! Daily budget (`DEFAULT_DAILY_REQUEST_LIMIT`, default 500) caps the
//! number of `execute_tool` calls per calendar day, preventing runaway
//! API usage during large initial backfills.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::source::run_gmail_sync;
use crate::openhuman::memory_sync::composio::providers::{
    pick_str, resolve_sync_interval_secs, ComposioProvider, CuratedTool, ProviderContext,
    ProviderUserProfile, SyncOutcome, SyncReason,
};

pub(super) const ACTION_GET_PROFILE: &str = "GMAIL_GET_PROFILE";
pub(super) const ACTION_FETCH_EMAILS: &str = "GMAIL_FETCH_EMAILS";

/// Base Gmail search query used on every sync pass.
///
/// Excludes spam and trash but intentionally does NOT restrict to `in:inbox` —
/// that restriction (issue #1713) prevented sent emails from ever being ingested.
/// Exported `pub(super)` so `tests.rs` can assert against the canonical value
/// rather than a duplicated literal.
pub(super) const BASE_QUERY: &str = "-in:spam -in:trash";

/// Gmail search query strings that retrieve sent mail.
///
/// Any of these can be passed as the `query` parameter to `GMAIL_FETCH_EMAILS`
/// to fetch outbound messages. Exported `pub(super)` for use in regression tests.
pub(super) const SENT_QUERIES: &[&str] = &["from:me", "label:SENT", "in:sent"];

pub struct GmailProvider;

impl GmailProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GmailProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for GmailProvider {
    fn toolkit_slug(&self) -> &'static str {
        "gmail"
    }

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(super::tools::GMAIL_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        Some(resolve_sync_interval_secs("gmail", 15 * 60))
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
            "[composio:gmail] fetch_user_profile via {ACTION_GET_PROFILE}"
        );

        let resp = ctx
            .execute(ACTION_GET_PROFILE, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:gmail] {ACTION_GET_PROFILE} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:gmail] {ACTION_GET_PROFILE}: {err}"));
        }

        // `data` is the inner Composio payload — paths here are relative
        // to it. (The previous `data.*` paths were dead — `pick_str`
        // does dotted-path traversal, so `data.emailAddress` looked for
        // a nested `data.data.emailAddress` that never exists.)
        let data = &resp.data;
        let email = pick_str(data, &["emailAddress", "email", "profile.emailAddress"]);
        // Don't fall back to the email when no name is returned — that
        // produces duplicated `display_name == email` rows in the
        // identity registry (#1365). Gmail's `GMAIL_GET_PROFILE` action
        // doesn't return a name today, so this stays None.
        let display_name = pick_str(data, &["name", "profile.name", "displayName"]);
        let profile_url = pick_str(
            data,
            &["display_url", "profileUrl", "profile_url", "profile.url"],
        );

        let profile = ProviderUserProfile {
            toolkit: "gmail".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email,
            username: None,
            avatar_url: None,
            profile_url,
            extras: data.clone(),
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
            "[composio:gmail] fetched user profile"
        );
        Ok(profile)
    }

    /// Incremental sync via the generic
    /// [`orchestrator`](crate::openhuman::memory_sync::composio::providers::orchestrator):
    /// pagination, dedup, the `max_items` cap, and cursor handling live in
    /// `run_sync`; the Gmail-specific primitives — the account-email preamble,
    /// server-side `after:` depth window, adaptive page ceiling, all-synced
    /// stop, and batch ingest — live in [`super::source`].
    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        run_gmail_sync(ctx, reason).await
    }

    async fn on_trigger(
        &self,
        ctx: &ProviderContext,
        trigger: &str,
        _payload: &Value,
    ) -> Result<(), String> {
        tracing::info!(
            connection_id = ?ctx.connection_id,
            trigger = %trigger,
            "[composio:gmail] on_trigger"
        );

        if trigger.eq_ignore_ascii_case("GMAIL_NEW_GMAIL_MESSAGE")
            || trigger.eq_ignore_ascii_case("GMAIL_NEW_MESSAGE")
        {
            if let Err(e) = self.sync(ctx, SyncReason::Manual).await {
                tracing::warn!(
                    error = %e,
                    "[composio:gmail] trigger-driven sync failed (non-fatal)"
                );
            }
        }
        Ok(())
    }
}

// The `max_items` cap math (`ItemCap`) lives in the orchestrator now that it is
// the sole consumer; the `sync_depth_days` date floor (`epoch_floor_from_depth`)
// stays in `super::super::helpers` because `gmail::source` builds an
// `after:<epoch>` filter from it.
