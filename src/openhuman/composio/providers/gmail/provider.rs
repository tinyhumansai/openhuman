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

use super::ingest::ingest_page_into_memory_tree;
use super::sync;
use crate::openhuman::composio::providers::sync_state::{extract_item_id, SyncState};
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

/// Adaptive page cap applied when a successful sync ran very recently.
/// If the previous sync wrote within
/// [`RECENT_SYNC_WINDOW_MS`], the upcoming sync is unlikely to need more
/// than a couple of pages — anything beyond that is almost certainly
/// re-fetching content `synced_ids` will throw away anyway.
const RECENT_SYNC_MAX_PAGES: u32 = 2;

/// "Recent" window used by the adaptive page cap. Five minutes is short
/// enough that periodic-tick churn and trigger-driven retries fall
/// inside it, but long enough that a genuine "no-activity" gap (e.g.
/// the user closing the laptop) drops back to the full
/// `MAX_PAGES_PER_SYNC` ceiling on the next wake.
const RECENT_SYNC_WINDOW_MS: u64 = 5 * 60 * 1000;

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

        // Fetch the account email up-front so every chunk gets a stable
        // per-account `source_id` (`gmail:{slug(email)}`). One HTTP
        // round-trip per sync; if it fails we fall back to the legacy
        // per-participants bucketing inside the ingest call so we
        // still write *something* useful.
        let account_email: Option<String> = match self.fetch_user_profile(ctx).await {
            Ok(profile) => profile.email,
            Err(e) => {
                tracing::warn!(
                    connection_id = %connection_id,
                    error = ?e,
                    "[composio:gmail] fetch_user_profile failed; ingest will fall back to per-participants source_id"
                );
                None
            }
        };

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

        // Adaptive page cap: if the previous successful sync wrote
        // within the recent window, cap pagination aggressively.
        // Initial backfills (`ConnectionCreated`) skip the cap — they
        // legitimately want the larger ceiling — and the cap only
        // kicks in when we have a prior `last_sync_at_ms` to compare
        // against, so first-ever syncs are unaffected.
        let max_pages = match reason {
            SyncReason::ConnectionCreated => MAX_PAGES_PER_SYNC,
            _ => match state.last_sync_at_ms {
                Some(last_ms) if sync::now_ms().saturating_sub(last_ms) < RECENT_SYNC_WINDOW_MS => {
                    tracing::debug!(
                        connection_id = %connection_id,
                        last_sync_at_ms = last_ms,
                        cap = RECENT_SYNC_MAX_PAGES,
                        "[composio:gmail] recent sync — applying adaptive page cap"
                    );
                    RECENT_SYNC_MAX_PAGES
                }
                _ => MAX_PAGES_PER_SYNC,
            },
        };

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut total_requests: u32 = 0;
        let mut newest_date: Option<String> = None;
        let mut newest_id: Option<String> = None;
        let mut page_token: Option<String> = None;
        let mut stop_reason: &'static str = "max_pages";

        for page_num in 0..max_pages {
            if state.budget_exhausted() {
                tracing::info!(
                    page = page_num,
                    "[composio:gmail] budget exhausted mid-sync, stopping pagination"
                );
                stop_reason = "budget_exhausted";
                break;
            }

            // Build the Gmail query. Prefer second-precision
            // `after:<unix>` over the old day-level `after:YYYY/MM/DD`
            // so same-day re-ticks do not re-fetch a whole day's
            // window every time. Fall back to the day filter only when
            // the cursor cannot be parsed as a timestamp.
            let mut query = "in:inbox -in:spam -in:trash".to_string();
            if let Some(ref cursor) = state.cursor {
                if let Some(epoch_filter) = sync::cursor_to_gmail_after_epoch_filter(cursor) {
                    query.push_str(&format!(" after:{epoch_filter}"));
                    tracing::debug!(
                        page = page_num,
                        filter = %epoch_filter,
                        "[composio:gmail] using epoch filter from cursor"
                    );
                } else if let Some(date_filter) = sync::cursor_to_gmail_after_filter(cursor) {
                    query.push_str(&format!(" after:{date_filter}"));
                    tracing::debug!(
                        page = page_num,
                        filter = %date_filter,
                        "[composio:gmail] using day-level filter from cursor (epoch parse failed)"
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

            let mut resp = ctx
                .client
                .execute_tool(ACTION_FETCH_EMAILS, Some(args.clone()))
                .await
                .map_err(|e| {
                    format!("[composio:gmail] {ACTION_FETCH_EMAILS} page {page_num}: {e:#}")
                })?;

            state.record_requests(1);
            total_requests += 1;

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

            // ── Step 4: pull the backend's pre-rendered `markdownFormatted`
            //    onto each message so the raw archive sees URL-shortened,
            //    footer-stripped output. Done BEFORE post_process so the
            //    reshape can pick up the per-message field. Then run the
            //    usual post-process which slims the envelope and feeds
            //    `extract_markdown_body` (which now prefers
            //    `markdownFormatted` per message).
            if let Some(top_md) = resp
                .markdown_formatted
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                super::post_process::apply_response_level_markdown(&mut resp.data, top_md);
            }
            self.post_process_action_result(ACTION_FETCH_EMAILS, Some(&args), &mut resp.data);

            let messages = sync::extract_messages(&resp.data);
            total_fetched += messages.len();

            if messages.is_empty() {
                tracing::debug!(
                    page = page_num,
                    "[composio:gmail] empty page, stopping pagination"
                );
                stop_reason = "empty_page";
                break;
            }

            // First-message early-stop: when the very first message of
            // the very first page matches the id we recorded at the
            // end of the previous sync, the inbox has not changed and
            // there is nothing left to fetch. Saves up to N-1 wasted
            // pages on quiet inboxes where the day-level filter would
            // otherwise re-fetch the same window.
            if page_num == 0 {
                let first_id = messages
                    .first()
                    .and_then(|m| extract_item_id(m, MESSAGE_ID_PATHS));
                if let (Some(seen), Some(first)) =
                    (state.last_seen_id.as_deref(), first_id.as_deref())
                {
                    if seen == first {
                        tracing::debug!(
                            connection_id = %connection_id,
                            first_id = %first,
                            "[composio:gmail] first page head matches last_seen_id — no new mail"
                        );
                        stop_reason = "head_unchanged";
                        // Capture the same id as the newest so the
                        // post-loop bookkeeping below keeps the
                        // `last_seen_id` field stable.
                        newest_id = Some(first.to_string());
                        break;
                    }
                }
            }

            // ── Step 5: filter against synced_ids for early-stop, advance
            //    cursor tracker, and collect new messages for batched
            //    memory-tree ingest. We collect candidate IDs to mark
            //    synced but defer the mark until the batch ingest returns
            //    Ok — otherwise a total ingest failure would leave these
            //    messages flagged as synced (gmail-side fetch dedup) but
            //    NOT in the memory tree, with no way to retry.
            let mut all_already_synced = true;
            let mut new_messages: Vec<Value> = Vec::with_capacity(messages.len());
            let mut pending_synced_ids: Vec<String> = Vec::with_capacity(messages.len());
            for (msg_index, msg) in messages.iter().enumerate() {
                // Track the newest date we've seen for cursor advancement,
                // independent of dedup status — we want the cursor to move
                // even if we've already ingested this page's content.
                if let Some(date_val) = extract_item_id(msg, MESSAGE_DATE_PATHS) {
                    if newest_date
                        .as_ref()
                        .is_none_or(|existing| date_val > *existing)
                    {
                        newest_date = Some(date_val);
                    }
                }

                let msg_id = extract_item_id(msg, MESSAGE_ID_PATHS);
                // Capture the very first id of page 0 as the
                // freshest-id-on-server marker for next-sync's
                // head-unchanged shortcut, regardless of dedup status.
                if page_num == 0 && msg_index == 0 {
                    if let Some(ref id) = msg_id {
                        newest_id = Some(id.clone());
                    }
                }
                if let Some(ref id) = msg_id {
                    if state.is_synced(id) {
                        continue;
                    }
                    pending_synced_ids.push(id.clone());
                }
                all_already_synced = false;
                new_messages.push(msg.clone());
            }

            // Single batched ingest into memory_tree. Chunk IDs are
            // content-hashed so re-ingest of the same message is an
            // idempotent UPSERT at the SQL layer; per-message dedup above
            // is purely an optimisation for the hot path.
            //
            // `synced_ids` here means "Gmail-side fetch dedup" (don't burn
            // API quota re-fetching this message), not "fully durable in
            // memory tree". We only commit those marks once the batch
            // returns Ok; on Err, nothing is marked, so the next sync
            // re-fetches and the chunk-id content hash handles dedup at
            // the storage layer.
            if !new_messages.is_empty() {
                let owner = format!("gmail-sync:{connection_id}");
                match ingest_page_into_memory_tree(
                    ctx.config.as_ref(),
                    &owner,
                    account_email.as_deref(),
                    &new_messages,
                )
                .await
                {
                    Ok(n) => {
                        for id in &pending_synced_ids {
                            state.mark_synced(id);
                        }
                        // total_persisted tracks messages, not chunks, for
                        // metric stability with the previous per-message
                        // persist path. n is the chunk count which we log
                        // for diagnostic purposes only.
                        total_persisted += new_messages.len();
                        tracing::debug!(
                            page = page_num,
                            new_messages = new_messages.len(),
                            ingested_chunks = n,
                            "[composio:gmail] page ingested into memory tree"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %format!("{e:#}"),
                            page = page_num,
                            new_messages = new_messages.len(),
                            "[composio:gmail] ingest_page_into_memory_tree failed (continuing)"
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
                stop_reason = "page_all_synced";
                break;
            }

            // Check for next page token.
            page_token = sync::extract_page_token(&resp.data);
            if page_token.is_none() {
                tracing::debug!(page = page_num, "[composio:gmail] no next page token, done");
                stop_reason = "no_more_pages";
                break;
            }
        }

        // ── Step 5: advance cursor and save state ───────────────────
        if let Some(new_cursor) = newest_date {
            state.advance_cursor(&new_cursor);
        }
        if let Some(ref freshest) = newest_id {
            state.set_last_seen_id(freshest);
        }
        let finished_at_ms = sync::now_ms();
        state.set_last_sync_at_ms(finished_at_ms);
        state.save(&memory).await?;

        // Bump the in-process scheduler timestamp so a periodic tick
        // does not immediately re-fire on top of a trigger-driven or
        // connection-created sync. Periodic itself already calls this
        // on its own success path; calling it from the provider keeps
        // the bookkeeping consistent for the other entry points.
        crate::openhuman::composio::periodic::record_sync_success(
            self.toolkit_slug(),
            &connection_id,
        );

        let dup_ratio = if total_fetched > 0 {
            (total_fetched.saturating_sub(total_persisted)) as f64 / total_fetched as f64
        } else {
            0.0
        };
        let summary = format!(
            "gmail sync ({reason}): fetched {total_fetched}, persisted {total_persisted} new, \
             requests {total_requests}, budget remaining {remaining}, stop={stop}",
            reason = reason.as_str(),
            remaining = state.budget_remaining(),
            stop = stop_reason,
        );
        tracing::info!(
            connection_id = %connection_id,
            reason = reason.as_str(),
            elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
            requests = total_requests,
            messages_total = total_fetched,
            messages_new = total_persisted,
            dup_ratio = dup_ratio,
            stop_reason = stop_reason,
            budget_remaining = state.budget_remaining(),
            adaptive_cap = max_pages != MAX_PAGES_PER_SYNC,
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
                "requests": total_requests,
                "budget_remaining": state.budget_remaining(),
                "cursor": state.cursor,
                "last_seen_id": state.last_seen_id,
                "stop_reason": stop_reason,
                "adaptive_cap": max_pages != MAX_PAGES_PER_SYNC,
                "dup_ratio": dup_ratio,
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
