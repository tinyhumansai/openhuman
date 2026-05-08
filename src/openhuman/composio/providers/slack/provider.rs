//! Composio-backed Slack provider.
//!
//! Drives Slack history ingestion **without** a user-managed bot token
//! — authorization lives in the user's Composio Slack connection, and
//! the actual API calls fan out through [`ComposioClient::execute_tool`]
//! against Composio's action catalog (`SLACK_LIST_CONVERSATIONS`,
//! `SLACK_FETCH_CONVERSATION_HISTORY`, `SLACK_FETCH_TEAM_INFO`, …).
//!
//! ## Per-sync lifecycle
//!
//! 1. Load [`SyncState`] for `(slack, connection_id)`. `state.cursor` is
//!    a JSON-encoded [`sync::ChannelCursors`] map — Slack needs a cursor
//!    per channel. Parse failures degrade to an empty map (full backfill),
//!    which is safe because chunk IDs are deterministic.
//! 2. Enumerate every channel the bot can read via
//!    [`ACTION_LIST_CONVERSATIONS`] with pagination.
//! 3. For each channel, pull messages since the per-channel cursor (or
//!    `now - BACKFILL_DAYS` if no cursor yet) via
//!    [`ACTION_FETCH_HISTORY`], paginated.
//! 4. Post-process each response via [`super::post_process`], enrich via
//!    [`super::sync::extract_messages`] to produce [`SlackMessage`]s with
//!    channel context and resolved user names.
//! 5. Ingest all collected messages via
//!    [`super::ingest::ingest_page_into_memory_tree`] — one `ingest_chat`
//!    call per message, no bucketing.
//! 6. Advance per-channel cursor to the latest successfully-ingested
//!    message's timestamp; save [`SyncState`].
//!
//! ## Idempotency
//!
//! Source id is `slack:{connection_id}` — stable per workspace. Chunk
//! IDs are content-hashed, so re-ingest is an UPSERT.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use super::ingest::ingest_page_into_memory_tree;
use super::sync;
use super::types::{SlackChannel, SlackMessage};
use super::users::SlackUsers;
use crate::openhuman::composio::client::ComposioClient;
use crate::openhuman::composio::providers::sync_state::SyncState;
use crate::openhuman::composio::providers::{
    pick_str, ComposioProvider, CuratedTool, ProviderContext, ProviderUserProfile, SyncOutcome,
    SyncReason,
};
use crate::openhuman::composio::types::ComposioExecuteResponse;

/// Composio action slug for channel listing.
const ACTION_LIST_CONVERSATIONS: &str = "SLACK_LIST_CONVERSATIONS";
/// Composio action slug for message history.
const ACTION_FETCH_HISTORY: &str = "SLACK_FETCH_CONVERSATION_HISTORY";
/// Composio action slug for team/workspace profile fetch.
const ACTION_FETCH_TEAM_INFO: &str = "SLACK_FETCH_TEAM_INFO";

/// Default backfill window (days) applied when a channel has no
/// cursor yet.
pub const BACKFILL_DAYS: i64 = 6;

/// Resolve the active backfill window in days. Reads
/// `OPENHUMAN_SLACK_BACKFILL_DAYS` env var if set and parseable as a
/// positive integer; falls back to [`BACKFILL_DAYS`] otherwise.
fn backfill_days() -> i64 {
    match std::env::var("OPENHUMAN_SLACK_BACKFILL_DAYS") {
        Ok(s) => match s.trim().parse::<i64>() {
            Ok(n) if n >= 1 => n,
            _ => {
                log::warn!(
                    "[composio:slack] OPENHUMAN_SLACK_BACKFILL_DAYS={s:?} not a positive integer; \
                     falling back to default {BACKFILL_DAYS}"
                );
                BACKFILL_DAYS
            }
        },
        Err(_) => BACKFILL_DAYS,
    }
}

/// Max channels listed per `SLACK_LIST_CONVERSATIONS` page.
const LIST_PAGE_SIZE: u32 = 200;

/// Max messages per `SLACK_FETCH_CONVERSATION_HISTORY` page.
const HISTORY_PAGE_SIZE: u32 = 1000;

/// Stop paginating any single channel's history after this many pages.
const MAX_HISTORY_PAGES_PER_CHANNEL: u32 = 20;

/// Stop paginating channel listings after this many pages.
const MAX_LIST_PAGES: u32 = 10;

/// Sync cadence — matches Gmail (15 minutes).
const SYNC_INTERVAL_SECS: u64 = 15 * 60;

/// Initial backoff for rate-limit retries.
const RATELIMIT_INITIAL_BACKOFF: Duration = Duration::from_secs(2);

/// Cap on per-retry backoff.
const RATELIMIT_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Total retries for a single rate-limited call before giving up.
const RATELIMIT_MAX_ATTEMPTS: u32 = 6;

/// Fixed inter-call sleep applied after every successful execute_tool.
const INTER_CALL_PACING: Duration = Duration::from_secs(20);

/// Resolve the JSON dump directory from `OPENHUMAN_SLACK_DUMP_DIR`.
fn dump_dir() -> Option<PathBuf> {
    std::env::var_os("OPENHUMAN_SLACK_DUMP_DIR").map(PathBuf::from)
}

/// Write a Composio response payload to disk under the dump dir. Best
/// effort — failures are logged at warn level and never fail the sync.
pub(super) fn dump_response(scope: &str, kind: &str, idx: u32, data: &Value) {
    let Some(base) = dump_dir() else {
        return;
    };
    let path = base.join(scope).join(format!("{kind}-{idx:04}.json"));
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(
                error = %e,
                path = %parent.display(),
                "[composio:slack] dump_response: create_dir_all failed (skipping dump)"
            );
            return;
        }
    }
    match serde_json::to_string_pretty(data) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "[composio:slack] dump_response: write failed"
                );
            } else {
                tracing::debug!(path = %path.display(), "[composio:slack] dumped response");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "[composio:slack] dump_response: serialize failed");
        }
    }
}

/// Wrap [`ComposioClient::execute_tool`] with rate-limit-aware retry +
/// inter-call pacing.
///
/// Returns `(response, attempts_made)` on first success so callers can
/// charge the daily quota meter for every attempt that hit Composio.
pub(super) async fn execute_with_retry(
    client: &ComposioClient,
    slug: &str,
    args: serde_json::Value,
    description: &str,
) -> Result<(ComposioExecuteResponse, u32), String> {
    let mut delay = RATELIMIT_INITIAL_BACKOFF;
    for attempt in 1..=RATELIMIT_MAX_ATTEMPTS {
        let resp = client
            .execute_tool(slug, Some(args.clone()))
            .await
            .map_err(|e| format!("{description}: {e:#}"))?;
        if resp.successful {
            tokio::time::sleep(INTER_CALL_PACING).await;
            return Ok((resp, attempt));
        }
        let err_str = resp.error.as_deref().unwrap_or("provider failure");
        let is_ratelimit = err_str.contains("ratelimited")
            || err_str.contains("rate_limit")
            || err_str.contains("rate limit");
        if is_ratelimit && attempt < RATELIMIT_MAX_ATTEMPTS {
            tracing::warn!(
                slug,
                attempt,
                max_attempts = RATELIMIT_MAX_ATTEMPTS,
                sleep_ms = delay.as_millis() as u64,
                "[composio:slack] rate-limited; backing off and retrying"
            );
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(RATELIMIT_MAX_BACKOFF);
            continue;
        }
        return Err(format!("{description}: {err_str}"));
    }
    Err(format!(
        "{description}: rate-limited after {RATELIMIT_MAX_ATTEMPTS} retries"
    ))
}

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
        Some(crate::openhuman::composio::providers::catalogs::SLACK_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        Some(SYNC_INTERVAL_SECS)
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
            "[composio:slack] fetch_user_profile via {ACTION_FETCH_TEAM_INFO}"
        );

        let resp = ctx
            .client
            .execute_tool(ACTION_FETCH_TEAM_INFO, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:slack] {ACTION_FETCH_TEAM_INFO} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:slack] {ACTION_FETCH_TEAM_INFO}: {err}"));
        }

        let data = &resp.data;
        let display_name = pick_str(data, &["data.team.name", "data.name", "team.name", "name"]);
        let profile_url = pick_str(data, &["data.team.url", "data.url", "team.url", "url"]);
        let email_domain = pick_str(
            data,
            &[
                "data.team.email_domain",
                "data.email_domain",
                "team.email_domain",
                "email_domain",
            ],
        );
        let avatar_url = pick_str(
            data,
            &[
                "data.team.icon.image_132",
                "data.team.icon.image_68",
                "team.icon.image_132",
            ],
        );

        let profile = ProviderUserProfile {
            toolkit: "slack".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email: None,
            username: None,
            avatar_url,
            profile_url,
            extras: json!({ "email_domain": email_domain, "raw": data }),
        };

        tracing::info!(
            connection_id = ?profile.connection_id,
            display_name = ?profile.display_name,
            "[composio:slack] fetched team info"
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
            "[composio:slack] sync starting"
        );

        let Some(memory) = ctx.memory_client() else {
            return Err("[composio:slack] memory client not ready".to_string());
        };
        let mut state = SyncState::load(&memory, "slack", &connection_id).await?;

        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:slack] daily request budget exhausted, skipping sync"
            );
            return Ok(SyncOutcome {
                toolkit: "slack".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "slack sync skipped: daily budget exhausted".to_string(),
                details: json!({ "budget_exhausted": true }),
            });
        }

        let mut cursors = sync::decode_cursors(state.cursor.as_deref());
        let now = chrono::Utc::now();

        // Pull the workspace user directory once per sync.
        let (users, user_call_count) = SlackUsers::fetch(&ctx.client).await;
        state.record_requests(user_call_count);
        tracing::info!(
            connection_id = %connection_id,
            user_count = users.len(),
            "[composio:slack] users cached for this sync"
        );

        // 1. Enumerate channels.
        let channels = list_all_channels(ctx, &mut state)
            .await
            .map_err(|e| format!("[composio:slack] list_channels: {e:#}"))?;

        tracing::info!(
            connection_id = %connection_id,
            channel_count = channels.len(),
            "[composio:slack] channels discovered"
        );

        let _ = state.save(&memory).await;

        let mut total_messages_ingested: usize = 0;
        let mut channels_processed: usize = 0;
        let mut channels_errored: usize = 0;

        // 2. Per-channel: fetch → post-process → enrich → ingest.
        for channel in &channels {
            if state.budget_exhausted() {
                tracing::warn!(
                    connection_id = %connection_id,
                    channel = %channel.id,
                    "[composio:slack] budget exhausted mid-sync, remaining channels deferred"
                );
                break;
            }

            match process_channel(
                ctx,
                &mut state,
                channel,
                &mut cursors,
                now,
                &users,
                &connection_id,
            )
            .await
            {
                Ok(n) => {
                    total_messages_ingested += n;
                    channels_processed += 1;
                }
                Err(err) => {
                    channels_errored += 1;
                    tracing::warn!(
                        connection_id = %connection_id,
                        channel = %channel.id,
                        error = %err,
                        "[composio:slack] channel sync failed (continuing with next channel)"
                    );
                }
            }

            state.advance_cursor(sync::encode_cursors(&cursors));
            if let Err(err) = state.save(&memory).await {
                tracing::warn!(
                    error = %err,
                    "[composio:slack] state save failed after channel (non-fatal)"
                );
            }
        }

        let finished_at_ms = sync::now_ms();
        let summary = format!(
            "slack sync: channels_processed={channels_processed} \
             channels_errored={channels_errored} \
             messages_ingested={total_messages_ingested}"
        );
        tracing::info!(
            connection_id = %connection_id,
            elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
            "{summary}"
        );

        Ok(SyncOutcome {
            toolkit: "slack".to_string(),
            connection_id: Some(connection_id),
            reason: reason.as_str().to_string(),
            items_ingested: total_messages_ingested,
            started_at_ms,
            finished_at_ms,
            summary,
            details: json!({
                "channels_processed": channels_processed,
                "channels_errored": channels_errored,
            }),
        })
    }

    async fn on_trigger(
        &self,
        ctx: &ProviderContext,
        trigger: &str,
        _payload: &Value,
    ) -> Result<(), String> {
        if trigger.to_ascii_uppercase().contains("MESSAGE") {
            if let Err(e) = self.sync(ctx, SyncReason::Manual).await {
                tracing::warn!(
                    error = %e,
                    "[composio:slack] trigger-driven sync failed (non-fatal)"
                );
            }
        }
        Ok(())
    }
}

/// Paginate through `SLACK_LIST_CONVERSATIONS` and flatten into a
/// single `Vec<SlackChannel>`.
async fn list_all_channels(
    ctx: &ProviderContext,
    state: &mut SyncState,
) -> Result<Vec<SlackChannel>, String> {
    let mut out: Vec<SlackChannel> = Vec::new();
    let mut cursor: Option<String> = None;

    for page_num in 0..MAX_LIST_PAGES {
        if state.budget_exhausted() {
            tracing::warn!(
                page = page_num,
                "[composio:slack] budget exhausted during channel listing"
            );
            break;
        }
        let mut args = json!({
            "types": "public_channel,private_channel",
            "exclude_archived": true,
            "limit": LIST_PAGE_SIZE,
        });
        if let Some(ref c) = cursor {
            args["cursor"] = json!(c);
        }

        let (mut resp, attempts) = execute_with_retry(
            &ctx.client,
            ACTION_LIST_CONVERSATIONS,
            args,
            &format!("{ACTION_LIST_CONVERSATIONS} page {page_num}"),
        )
        .await?;
        state.record_requests(attempts);
        dump_response("_meta", "channels", page_num, &resp.data);

        // Post-process then enrich.
        super::post_process::post_process(ACTION_LIST_CONVERSATIONS, None, &mut resp.data);
        out.extend(sync::extract_channels(&resp.data));
        cursor = sync::extract_next_cursor(&resp.data);
        if cursor.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Pull one channel's history since its cursor, post-process + enrich each
/// page, then ingest all messages. Returns the number of chunks written.
async fn process_channel(
    ctx: &ProviderContext,
    state: &mut SyncState,
    channel: &SlackChannel,
    cursors: &mut sync::ChannelCursors,
    now: chrono::DateTime<chrono::Utc>,
    users: &SlackUsers,
    connection_id: &str,
) -> Result<usize, String> {
    let oldest_secs = cursors
        .get(&channel.id)
        .copied()
        .unwrap_or_else(|| (now - chrono::Duration::days(backfill_days())).timestamp());

    let mut all_messages: Vec<SlackMessage> = Vec::new();
    let mut cursor: Option<String> = None;

    for page_num in 0..MAX_HISTORY_PAGES_PER_CHANNEL {
        if state.budget_exhausted() {
            tracing::warn!(
                channel = %channel.id,
                page = page_num,
                "[composio:slack] budget exhausted during history fetch"
            );
            break;
        }

        let mut args = json!({
            "channel": channel.id,
            "oldest": format!("{oldest_secs}.000000"),
            "inclusive": false,
            "limit": HISTORY_PAGE_SIZE,
        });
        if let Some(ref c) = cursor {
            args["cursor"] = json!(c);
        }

        let (mut resp, attempts) = execute_with_retry(
            &ctx.client,
            ACTION_FETCH_HISTORY,
            args,
            &format!(
                "{ACTION_FETCH_HISTORY} channel={} page {page_num}",
                channel.id
            ),
        )
        .await?;
        state.record_requests(attempts);
        dump_response(&channel.id, "history", page_num, &resp.data);

        // Post-process to slim envelope, then enrich with channel context + users.
        super::post_process::post_process(ACTION_FETCH_HISTORY, None, &mut resp.data);
        let msgs = sync::extract_messages(&resp.data, channel, users);
        tracing::debug!(
            channel = %channel.id,
            page = page_num,
            fetched = msgs.len(),
            "[composio:slack] history page"
        );
        if msgs.is_empty() {
            break;
        }
        all_messages.extend(msgs);
        cursor = sync::extract_next_cursor(&resp.data);
        if cursor.is_none() {
            break;
        }
    }

    if all_messages.is_empty() {
        tracing::debug!(
            channel = %channel.id,
            "[composio:slack] no new messages"
        );
        return Ok(0);
    }

    let msg_count = all_messages.len();
    tracing::info!(
        channel = %channel.id,
        messages = msg_count,
        "[composio:slack] ingesting channel messages"
    );

    match ingest_page_into_memory_tree(&ctx.config, "", connection_id, &all_messages).await {
        Ok(chunks) => {
            // Advance cursor to the latest successfully-ingested message timestamp.
            if let Some(latest) = all_messages.iter().map(|m| m.timestamp.timestamp()).max() {
                cursors.insert(channel.id.clone(), latest);
            }
            tracing::info!(
                channel = %channel.id,
                messages = msg_count,
                chunks,
                "[composio:slack] channel ingest done"
            );
            Ok(chunks)
        }
        Err(e) => {
            tracing::warn!(
                channel = %channel.id,
                error = %e,
                "[composio:slack] ingest_page_into_memory_tree failed (cursor not advanced)"
            );
            // Don't advance cursor — next sync re-fetches this range.
            Err(format!("ingest failed for channel {}: {e:#}", channel.id))
        }
    }
}

// ── Search-based backfill (one-shot) ────────────────────────────────

/// Composio action slug for workspace-wide message search.
const ACTION_SEARCH_MESSAGES: &str = "SLACK_SEARCH_MESSAGES";

/// Max matches per `SLACK_SEARCH_MESSAGES` page.
const SEARCH_PAGE_SIZE: u32 = 100;

/// Hard cap on pages walked per backfill run.
const MAX_SEARCH_PAGES: u32 = 50;

/// Run a one-shot historical backfill via `SLACK_SEARCH_MESSAGES` —
/// workspace-wide paginated search instead of per-channel
/// `conversations.history`. Each successful call returns matches across
/// many channels, so partial progress translates to real coverage.
///
/// Designed for the `slack-backfill` bin specifically — the periodic
/// `SlackProvider::sync()` keeps the per-channel incremental path.
///
/// Lifecycle:
/// 1. Cache the channel directory and user directory.
/// 2. Paginate `SLACK_SEARCH_MESSAGES` until exhausted or page cap.
/// 3. Group messages by channel_id, ingest each group via
///    `ingest_page_into_memory_tree`. No bucketing.
pub async fn run_backfill_via_search(
    ctx: &ProviderContext,
    backfill_days: i64,
) -> Result<SyncOutcome, String> {
    let started_at_ms = sync::now_ms();
    let connection_id = ctx
        .connection_id
        .clone()
        .unwrap_or_else(|| "default".to_string());

    tracing::info!(
        connection_id = %connection_id,
        backfill_days,
        "[composio:slack] search-based backfill starting"
    );

    let memory = ctx
        .memory_client()
        .ok_or_else(|| "[composio:slack] memory client not ready".to_string())?;
    let mut state = SyncState::load(&memory, "slack", &connection_id).await?;

    if state.budget_exhausted() {
        return Ok(SyncOutcome {
            toolkit: "slack".to_string(),
            connection_id: Some(connection_id),
            reason: SyncReason::Manual.as_str().to_string(),
            items_ingested: 0,
            started_at_ms,
            finished_at_ms: sync::now_ms(),
            summary: "slack search-backfill skipped: daily budget exhausted".to_string(),
            details: json!({ "budget_exhausted": true }),
        });
    }

    // 1. Channel directory.
    let channels = list_all_channels(ctx, &mut state)
        .await
        .map_err(|e| format!("[composio:slack] list_channels: {e:#}"))?;
    let channel_map: HashMap<String, SlackChannel> =
        channels.into_iter().map(|c| (c.id.clone(), c)).collect();

    // 2. User directory.
    let (users, user_call_count) = SlackUsers::fetch(&ctx.client).await;
    state.record_requests(user_call_count);
    tracing::info!(
        connection_id = %connection_id,
        user_count = users.len(),
        channel_count = channel_map.len(),
        "[composio:slack] caches ready"
    );
    let _ = state.save(&memory).await;

    // 3. Paginated workspace-wide search.
    let now = chrono::Utc::now();
    let after = (now - chrono::Duration::days(backfill_days))
        .format("%Y-%m-%d")
        .to_string();
    let query = format!("after:{after}");
    let mut all_messages: Vec<SlackMessage> = Vec::new();
    let mut page: u32 = 1;
    let mut total_pages: u32 = 1;

    loop {
        if state.budget_exhausted() {
            tracing::warn!(
                page,
                "[composio:slack] budget exhausted mid-search, halting"
            );
            break;
        }
        let args = json!({
            "query": query,
            "count": SEARCH_PAGE_SIZE,
            "sort": "timestamp",
            "sort_dir": "asc",
            "page": page,
        });
        let (mut resp, attempts) = execute_with_retry(
            &ctx.client,
            ACTION_SEARCH_MESSAGES,
            args,
            &format!("{ACTION_SEARCH_MESSAGES} page {page}"),
        )
        .await?;
        state.record_requests(attempts);
        dump_response("_meta", "search", page, &resp.data);

        // Post-process, then enrich with channel_map + users.
        super::post_process::post_process(ACTION_SEARCH_MESSAGES, None, &mut resp.data);
        let msgs = sync::extract_search_messages(&resp.data, &channel_map, &users);
        if page == 1 {
            total_pages = sync::extract_search_total_pages(&resp.data).min(MAX_SEARCH_PAGES);
            tracing::info!(
                connection_id = %connection_id,
                total_pages,
                first_page_msgs = msgs.len(),
                "[composio:slack] search pagination plan"
            );
        }
        let fetched = msgs.len();
        all_messages.extend(msgs);
        if fetched == 0 || page >= total_pages {
            break;
        }
        page += 1;
    }
    let _ = state.save(&memory).await;

    // 4. Group by channel_id and ingest each group.
    let buckets = super::ingest::bucket_by_channel(&all_messages);
    let channel_count = buckets.len();
    tracing::info!(
        connection_id = %connection_id,
        channels = channel_count,
        total_messages = all_messages.len(),
        "[composio:slack] grouped messages by channel for ingest"
    );

    let mut channels_flushed = 0usize;
    let mut channels_failed = 0usize;
    let mut total_chunks = 0usize;

    for (channel_id, msgs_for_channel) in &buckets {
        let page: Vec<SlackMessage> = msgs_for_channel.iter().map(|m| (*m).clone()).collect();
        match ingest_page_into_memory_tree(&ctx.config, "", &connection_id, &page).await {
            Ok(chunks) => {
                channels_flushed += 1;
                total_chunks += chunks;
                tracing::info!(
                    channel = %channel_id,
                    messages = page.len(),
                    chunks,
                    "[composio:slack] search-backfill channel ingested"
                );
            }
            Err(err) => {
                channels_failed += 1;
                tracing::warn!(
                    channel = %channel_id,
                    error = %err,
                    "[composio:slack] search-backfill ingest failed"
                );
            }
        }
    }

    let finished_at_ms = sync::now_ms();
    let summary = format!(
        "slack search-backfill: pages={page} channels_flushed={channels_flushed} \
         channels_failed={channels_failed} chunks={total_chunks}"
    );
    tracing::info!(
        connection_id = %connection_id,
        elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
        "{summary}"
    );

    Ok(SyncOutcome {
        toolkit: "slack".to_string(),
        connection_id: Some(connection_id),
        reason: SyncReason::Manual.as_str().to_string(),
        items_ingested: total_chunks,
        started_at_ms,
        finished_at_ms,
        summary,
        details: json!({
            "pages_walked": page,
            "channels_flushed": channels_flushed,
            "channels_failed": channels_failed,
            "total_chunks": total_chunks,
        }),
    })
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
