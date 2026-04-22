//! Gmail provider — incremental sync with per-item persistence.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Fetch a page of recent messages via `GMAIL_FETCH_EMAILS`, adding
//!      a date filter when a cursor exists so only newer mail is returned.
//!   4. Deduplicate against `synced_ids` in the state.
//!   5. Persist each **new** message as its own memory document (not a
//!      single giant snapshot) so agent recall can find individual emails.
//!   6. Paginate (up to budget) until no more results or all items in the
//!      page are already synced.
//!   7. Advance the cursor and save state.
//!
//! Daily budget (`DEFAULT_DAILY_REQUEST_LIMIT`, default 500) caps the
//! number of `execute_tool` calls per calendar day, preventing runaway
//! API usage during large initial backfills.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::sync;
use crate::openhuman::composio::providers::sync_state::{
    extract_item_id, persist_single_item, SyncState,
};
use crate::openhuman::composio::providers::{
    pick_str, ComposioProvider, CuratedTool, ProviderContext, ProviderUserProfile, SyncOutcome,
    SyncReason,
};

const ACTION_GET_PROFILE: &str = "GMAIL_GET_PROFILE";
const ACTION_FETCH_EMAILS: &str = "GMAIL_FETCH_EMAILS";

/// Page size per API call. Kept moderate so each call is fast and we
/// get frequent checkpoints for the daily budget.
const PAGE_SIZE: u32 = 25;

/// Larger page size for the very first sync after OAuth so the user
/// gets a meaningful initial snapshot.
const INITIAL_PAGE_SIZE: u32 = 50;

/// Maximum pages to fetch in a single sync pass (guards against infinite
/// pagination loops). Combined with PAGE_SIZE this yields at most
/// 500 items per sync pass, well within the daily budget.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Paths to try when extracting a message's unique ID from the Composio
/// response envelope.
const MESSAGE_ID_PATHS: &[&str] = &["id", "data.id", "messageId", "data.messageId"];

/// Paths for extracting the internal date (epoch millis or date string)
/// used as the sync cursor.
const MESSAGE_DATE_PATHS: &[&str] = &[
    "internalDate",
    "data.internalDate",
    "date",
    "data.date",
    "receivedAt",
    "data.receivedAt",
];

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
        Some(15 * 60)
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
            .client
            .execute_tool(ACTION_GET_PROFILE, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:gmail] {ACTION_GET_PROFILE} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:gmail] {ACTION_GET_PROFILE}: {err}"));
        }

        let data = &resp.data;
        let email = pick_str(
            data,
            &[
                "data.emailAddress",
                "data.email",
                "emailAddress",
                "email",
                "data.profile.emailAddress",
            ],
        );
        let display_name = pick_str(
            data,
            &[
                "data.name",
                "data.profile.name",
                "name",
                "displayName",
                "data.displayName",
            ],
        )
        .or_else(|| email.clone());
        let profile_url = pick_str(
            data,
            &[
                "data.profileUrl",
                "data.profile_url",
                "data.profile.url",
                "profileUrl",
                "profile_url",
            ],
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

    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        let started_at_ms = sync::now_ms();
        let connection_id = ctx
            .connection_id
            .clone()
            .unwrap_or_else(|| "default".to_string());

        tracing::info!(
            connection_id = %connection_id,
            reason = reason.as_str(),
            "[composio:gmail] incremental sync starting"
        );

        // ── Step 1: load persistent sync state ──────────────────────
        let Some(memory) = ctx.memory_client() else {
            return Err("[composio:gmail] memory client not ready".to_string());
        };
        let mut state = SyncState::load(&memory, "gmail", &connection_id).await?;

        // ── Step 2: check daily budget ──────────────────────────────
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:gmail] daily request budget exhausted, skipping sync"
            );
            return Ok(SyncOutcome {
                toolkit: "gmail".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "gmail sync skipped: daily budget exhausted".to_string(),
                details: json!({ "budget_exhausted": true }),
            });
        }

        // ── Step 3: paginated incremental fetch ─────────────────────
        let page_size = match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        };

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut newest_date: Option<String> = None;
        let mut page_token: Option<String> = None;

        for page_num in 0..MAX_PAGES_PER_SYNC {
            if state.budget_exhausted() {
                tracing::info!(
                    page = page_num,
                    "[composio:gmail] budget exhausted mid-sync, stopping pagination"
                );
                break;
            }

            // Build the Gmail query. If we have a cursor (date of last
            // synced message), add `after:YYYY/MM/DD` so the API only
            // returns newer mail.
            let mut query = "in:inbox -in:spam -in:trash".to_string();
            if let Some(ref cursor) = state.cursor {
                if let Some(date_filter) = sync::cursor_to_gmail_after_filter(cursor) {
                    query.push_str(&format!(" after:{date_filter}"));
                    tracing::debug!(
                        page = page_num,
                        filter = %date_filter,
                        "[composio:gmail] using date filter from cursor"
                    );
                }
            }

            let mut args = json!({
                "max_results": page_size,
                "query": query,
            });
            if let Some(ref token) = page_token {
                args["page_token"] = json!(token);
            }

            let resp = ctx
                .client
                .execute_tool(ACTION_FETCH_EMAILS, Some(args))
                .await
                .map_err(|e| {
                    format!("[composio:gmail] {ACTION_FETCH_EMAILS} page {page_num}: {e:#}")
                })?;

            state.record_requests(1);

            if !resp.successful {
                let err = resp
                    .error
                    .clone()
                    .unwrap_or_else(|| "provider reported failure".to_string());
                // Save state so budget accounting isn't lost.
                let _ = state.save(&memory).await;
                return Err(format!(
                    "[composio:gmail] {ACTION_FETCH_EMAILS} page {page_num}: {err}"
                ));
            }

            let messages = sync::extract_messages(&resp.data);
            total_fetched += messages.len();

            if messages.is_empty() {
                tracing::debug!(
                    page = page_num,
                    "[composio:gmail] empty page, stopping pagination"
                );
                break;
            }

            // ── Step 4: deduplicate and persist per-item ────────────
            let mut all_already_synced = true;
            for msg in &messages {
                let Some(msg_id) = extract_item_id(msg, MESSAGE_ID_PATHS) else {
                    tracing::debug!("[composio:gmail] message missing ID, skipping");
                    continue;
                };

                // Track the newest date we've seen for cursor advancement.
                if let Some(date_val) = extract_item_id(msg, MESSAGE_DATE_PATHS) {
                    if newest_date
                        .as_ref()
                        .is_none_or(|existing| date_val > *existing)
                    {
                        newest_date = Some(date_val);
                    }
                }

                if state.is_synced(&msg_id) {
                    continue;
                }
                all_already_synced = false;

                // Build a human-readable title for this email document.
                let subject = pick_str(
                    msg,
                    &[
                        "subject",
                        "data.subject",
                        "payload.headers.Subject",
                        "snippet",
                    ],
                )
                .unwrap_or_else(|| format!("Email {msg_id}"));
                let doc_id = format!("composio-gmail-msg-{msg_id}");
                let title = format!("Gmail: {subject}");

                match persist_single_item(
                    &memory,
                    "gmail",
                    &doc_id,
                    &title,
                    msg,
                    "gmail",
                    ctx.connection_id.as_deref(),
                )
                .await
                {
                    Ok(_) => {
                        state.mark_synced(&msg_id);
                        total_persisted += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            msg_id = %msg_id,
                            error = %e,
                            "[composio:gmail] failed to persist message (continuing)"
                        );
                    }
                }
            }

            // If every message in this page was already synced, there's
            // nothing new beyond this point — stop paginating.
            if all_already_synced {
                tracing::debug!(
                    page = page_num,
                    "[composio:gmail] all items in page already synced, stopping"
                );
                break;
            }

            // Check for next page token.
            page_token = sync::extract_page_token(&resp.data);
            if page_token.is_none() {
                tracing::debug!(page = page_num, "[composio:gmail] no next page token, done");
                break;
            }
        }

        // ── Step 5: advance cursor and save state ───────────────────
        if let Some(new_cursor) = newest_date {
            state.advance_cursor(&new_cursor);
        }
        state.save(&memory).await?;

        let finished_at_ms = sync::now_ms();
        let summary = format!(
            "gmail sync ({reason}): fetched {total_fetched}, persisted {total_persisted} new, \
             budget remaining {remaining}",
            reason = reason.as_str(),
            remaining = state.budget_remaining(),
        );
        tracing::info!(
            connection_id = %connection_id,
            elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
            total_fetched,
            total_persisted,
            budget_remaining = state.budget_remaining(),
            "[composio:gmail] incremental sync complete"
        );

        Ok(SyncOutcome {
            toolkit: "gmail".to_string(),
            connection_id: Some(connection_id),
            reason: reason.as_str().to_string(),
            items_ingested: total_persisted,
            started_at_ms,
            finished_at_ms,
            summary,
            details: json!({
                "messages_fetched": total_fetched,
                "messages_persisted": total_persisted,
                "budget_remaining": state.budget_remaining(),
                "cursor": state.cursor,
                "synced_ids_total": state.synced_ids.len(),
            }),
        })
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
