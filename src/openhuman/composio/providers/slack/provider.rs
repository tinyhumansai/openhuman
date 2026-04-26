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
//!    per channel, not one global watermark (Gmail's single `cursor`
//!    string wouldn't cut it). Parse failures degrade to an empty map,
//!    i.e. full backfill, which is safe because chunk IDs are
//!    deterministic.
//! 2. Enumerate every channel the bot can read via
//!    [`ACTION_LIST_CONVERSATIONS`] with pagination.
//! 3. For each channel, pull messages since the per-channel cursor (or
//!    `now - BACKFILL_DAYS` if no cursor yet) via
//!    [`ACTION_FETCH_HISTORY`], paginated.
//! 4. Hand every collected message to
//!    [`slack_ingestion::bucketer::split_closed`] — produces closed
//!    6-hour UTC buckets + a "still open" remainder (discarded; the
//!    next sync will re-fetch that window, which is cheap because we
//!    advance the cursor **only** after a bucket flushes).
//! 5. Ingest each closed bucket via
//!    [`slack_ingestion::ops::ingest_bucket`] — canonicalise → chunk →
//!    score (with the config's Ollama entity extractor, if set) →
//!    persist → seal cascade (with the config's Ollama summariser).
//! 6. Advance per-channel cursor to the latest flushed bucket's end
//!    timestamp; save [`SyncState`].
//!
//! ## Idempotency
//!
//! - `source_id = "slack:<channel>:<bucket_start_epoch>"` is stable
//!   across runs, so chunk IDs are deterministic and re-ingest is an
//!   UPSERT — no duplicates.
//! - The cursor advances **only** after a bucket's `ingest_bucket`
//!   call returns `Ok`. A crash mid-fetch means the next run re-walks
//!   that range; a crash mid-ingest re-fetches the range too. Both are
//!   safe by the chunk-id property above.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use super::sync;
use super::users::SlackUsers;
use crate::openhuman::composio::client::ComposioClient;
use crate::openhuman::composio::providers::sync_state::SyncState;
use crate::openhuman::composio::providers::{
    pick_str, ComposioProvider, CuratedTool, ProviderContext, ProviderUserProfile, SyncOutcome,
    SyncReason,
};
use crate::openhuman::composio::types::ComposioExecuteResponse;
use crate::openhuman::memory::slack_ingestion::bucketer::{
    bucket_end_for, split_closed, GRACE_PERIOD,
};
use crate::openhuman::memory::slack_ingestion::ops::ingest_bucket;
use crate::openhuman::memory::slack_ingestion::types::{SlackChannel, SlackMessage};

/// Composio action slug for channel listing.
const ACTION_LIST_CONVERSATIONS: &str = "SLACK_LIST_CONVERSATIONS";
/// Composio action slug for message history.
const ACTION_FETCH_HISTORY: &str = "SLACK_FETCH_CONVERSATION_HISTORY";
/// Composio action slug for team/workspace profile fetch.
const ACTION_FETCH_TEAM_INFO: &str = "SLACK_FETCH_TEAM_INFO";

/// Default backfill window (days) applied when a channel has no
/// cursor yet. Override via `OPENHUMAN_SLACK_BACKFILL_DAYS` env var
/// when a different window is needed (e.g. 30 days for a fresh
/// workspace, or 1 day for fast smoke tests).
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

/// Max channels listed per `SLACK_LIST_CONVERSATIONS` page. Slack caps
/// this at 1000; 200 is a safe default.
const LIST_PAGE_SIZE: u32 = 200;

/// Max messages per `SLACK_FETCH_CONVERSATION_HISTORY` page. With
/// `INTER_CALL_PACING` clamping us to 3 req/min, the marginal cost of
/// asking for 1000 vs 200 per call is just larger response payloads —
/// and we want to drain the 30-day window in as few calls as possible
/// to minimise total quota burn.
const HISTORY_PAGE_SIZE: u32 = 1000;

/// Stop paginating any single channel's history after this many pages
/// so one dormant backfill can't consume the whole daily budget. With
/// `HISTORY_PAGE_SIZE=200`, this yields ≤ 4000 messages per channel per
/// sync — the next tick picks up the rest incrementally.
const MAX_HISTORY_PAGES_PER_CHANNEL: u32 = 20;

/// Stop paginating channel listings after this many pages.
const MAX_LIST_PAGES: u32 = 10;

/// Sync cadence — matches Gmail (15 minutes). Bucket flush granularity
/// is 6 hours anyway, so tighter cadences just burn API calls.
const SYNC_INTERVAL_SECS: u64 = 15 * 60;

/// Initial backoff for rate-limit retries. Slack tier-2 endpoints
/// (`conversations.history`, `users.list`) advertise ~50 req/min but
/// Composio's quota appears stricter; 2s gives the bucket time to
/// refill on a `ratelimited` response.
const RATELIMIT_INITIAL_BACKOFF: Duration = Duration::from_secs(2);

/// Cap on per-retry backoff. Without a ceiling, exponential growth
/// would push individual retries into the multi-minute range and stall
/// the whole sync.
const RATELIMIT_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Total retries for a single rate-limited call before giving up. With
/// 2s → 4 → 8 → 16 → 30 → 30 backoff this gives ≤ 90s of grace per
/// call; longer than the worst Composio rate-window we've observed.
const RATELIMIT_MAX_ATTEMPTS: u32 = 6;

/// Fixed inter-call sleep applied after every successful execute_tool.
/// At 20s per call we stay at 3 req/min — well below Slack's tier-2
/// limit of 50/min and conservative enough to ride out Composio's
/// stricter staging-tier quota replenishment. Trade-off: a full
/// 30-day backfill across 11 channels takes 20-30 min wall-clock.
const INTER_CALL_PACING: Duration = Duration::from_secs(20);

/// Resolve the JSON dump directory from `OPENHUMAN_SLACK_DUMP_DIR`.
/// When unset, dumping is disabled. When set, every successful Composio
/// response is mirrored to `<dir>/<scope>/<kind>-<idx>.json` so the
/// raw payload can be replayed into `ingest_chat` later without
/// re-burning quota.
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
/// inter-call pacing. Returns the underlying response on the first
/// successful attempt; bails out with a useful error otherwise. The
/// retry only fires for `ok=false, error=ratelimited` responses; other
/// failures pass through unchanged so callers can decide how to react.
async fn execute_with_retry(
    client: &ComposioClient,
    slug: &str,
    args: serde_json::Value,
    description: &str,
) -> Result<ComposioExecuteResponse, String> {
    let mut delay = RATELIMIT_INITIAL_BACKOFF;
    for attempt in 1..=RATELIMIT_MAX_ATTEMPTS {
        let resp = client
            .execute_tool(slug, Some(args.clone()))
            .await
            .map_err(|e| format!("{description}: {e:#}"))?;
        if resp.successful {
            // Pace the next call so we don't immediately re-trip.
            tokio::time::sleep(INTER_CALL_PACING).await;
            return Ok(resp);
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
            email: None, // Slack team_info is workspace-scoped, not user-scoped
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

        // Pull the workspace user directory once per sync so author
        // ids and `<@…>` mentions in message text canonicalise to
        // human-readable names. Soft-fails to an empty cache; raw ids
        // simply pass through in that case.
        let users = SlackUsers::fetch(&ctx.client).await;
        state.record_requests(1);
        tracing::info!(
            connection_id = %connection_id,
            user_count = users.len(),
            "[composio:slack] users cached for this sync"
        );

        // 1. Enumerate channels ────────────────────────────────────────
        let channels = list_all_channels(ctx, &mut state)
            .await
            .map_err(|e| format!("[composio:slack] list_channels: {e:#}"))?;

        tracing::info!(
            connection_id = %connection_id,
            channel_count = channels.len(),
            "[composio:slack] channels discovered"
        );

        // Save budget state early so a panic mid-fetch doesn't leak the
        // request counter. Cheap.
        let _ = state.save(&memory).await;

        let mut total_flushed_buckets: usize = 0;
        let mut channels_processed: usize = 0;
        let mut channels_errored: usize = 0;

        // 2. Per-channel: fetch → bucket → ingest → advance cursor ────
        for channel in &channels {
            if state.budget_exhausted() {
                tracing::warn!(
                    connection_id = %connection_id,
                    channel = %channel.id,
                    "[composio:slack] budget exhausted mid-sync, remaining channels deferred"
                );
                break;
            }

            match process_channel(ctx, &mut state, channel, &mut cursors, now, &users).await {
                Ok(n) => {
                    total_flushed_buckets += n;
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

            // Save after every channel — a crash between channels
            // shouldn't lose already-advanced cursors.
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
             buckets_flushed={total_flushed_buckets}"
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
            items_ingested: total_flushed_buckets,
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
        // Slack trigger names use SLACK_RECEIVE_MESSAGE / similar —
        // match loosely so future slug changes don't silently drop this.
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
/// single `Vec<SlackChannel>`. Aborts early if the daily budget runs out
/// partway through — callers handle the partial list gracefully.
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

        let resp = execute_with_retry(
            &ctx.client,
            ACTION_LIST_CONVERSATIONS,
            args,
            &format!("{ACTION_LIST_CONVERSATIONS} page {page_num}"),
        )
        .await?;
        state.record_requests(1);
        dump_response("_meta", "channels", page_num, &resp.data);

        out.extend(sync::extract_channels(&resp.data));
        cursor = sync::extract_next_cursor(&resp.data);
        if cursor.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Pull one channel's history since its cursor, bucket it, and ingest
/// every closed bucket. Returns the number of buckets actually flushed.
async fn process_channel(
    ctx: &ProviderContext,
    state: &mut SyncState,
    channel: &SlackChannel,
    cursors: &mut sync::ChannelCursors,
    now: chrono::DateTime<chrono::Utc>,
    users: &SlackUsers,
) -> Result<usize, String> {
    // Derive `oldest` — per-channel cursor if we've synced before,
    // else `now - backfill_days()` for a fresh channel.
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

        let resp = execute_with_retry(
            &ctx.client,
            ACTION_FETCH_HISTORY,
            args,
            &format!(
                "{ACTION_FETCH_HISTORY} channel={} page {page_num}",
                channel.id
            ),
        )
        .await?;
        state.record_requests(1);
        dump_response(&channel.id, "history", page_num, &resp.data);

        let msgs = sync::extract_messages(&resp.data, &channel.id, users);
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

    // Bucket — closed buckets ingest now; still-open ones get discarded
    // and will be re-fetched on the next tick (cursor only advances on
    // successful flush).
    let (closed, remaining) = split_closed(all_messages, now, GRACE_PERIOD);
    tracing::debug!(
        channel = %channel.id,
        closed_buckets = closed.len(),
        remaining_msgs = remaining.len(),
        "[composio:slack] bucket split"
    );

    let mut flushed = 0usize;
    let mut latest_end: Option<chrono::DateTime<chrono::Utc>> = None;
    let connection_id = ctx.connection_id.as_deref().unwrap_or("default");

    for bucket in closed {
        match ingest_bucket(&ctx.config, channel, &bucket, "", connection_id).await {
            Ok(res) => {
                flushed += 1;
                latest_end = Some(bucket.end);
                tracing::info!(
                    channel = %channel.id,
                    bucket_start = %bucket.start.to_rfc3339(),
                    chunks_written = res.chunks_written,
                    "[composio:slack] ingested bucket"
                );
            }
            Err(err) => {
                tracing::warn!(
                    channel = %channel.id,
                    bucket_start = %bucket.start.to_rfc3339(),
                    error = %err,
                    "[composio:slack] ingest failed (cursor not advanced for this bucket)"
                );
                // Stop processing more buckets in this channel — the
                // next run will re-fetch from the current cursor.
                break;
            }
        }
    }

    if let Some(end) = latest_end {
        cursors.insert(channel.id.clone(), end.timestamp());
    }
    // Ensure bucket_end_for stays referenced (future-proofing for
    // cascade-aware flushes) without dead-code.
    let _ = bucket_end_for;

    Ok(flushed)
}

// ── Search-based backfill (one-shot) ────────────────────────────────

/// Composio action slug for workspace-wide message search.
const ACTION_SEARCH_MESSAGES: &str = "SLACK_SEARCH_MESSAGES";

/// Max matches per `SLACK_SEARCH_MESSAGES` page (Slack's documented cap).
const SEARCH_PAGE_SIZE: u32 = 100;

/// Hard cap on pages walked per backfill run. With 100 matches/page
/// that's 5000 messages — plenty for typical 30-day windows on small
/// workspaces. Larger backfills can run multiple times against
/// successive sub-windows.
const MAX_SEARCH_PAGES: u32 = 50;

/// Run a one-shot historical backfill via `SLACK_SEARCH_MESSAGES` —
/// workspace-wide paginated search instead of per-channel
/// `conversations.history`. Better quota efficiency: each successful
/// call returns matches across many channels at once, so partial
/// progress translates to real coverage instead of one channel's
/// worth.
///
/// Designed for the `slack-backfill` bin specifically — the periodic
/// `SlackProvider::sync()` keeps the per-channel incremental path so
/// it stays cheap on each tick.
///
/// Lifecycle:
/// 1. Cache the channel directory (one `SLACK_LIST_CONVERSATIONS` call,
///    paginated) so canonicalisation can label channels by name + know
///    private-vs-public.
/// 2. Cache the user directory (one `SLACK_LIST_ALL_USERS` paginated
///    walk via [`SlackUsers::fetch`]).
/// 3. Paginate `SLACK_SEARCH_MESSAGES` with
///    `query = "after:<YYYY-MM-DD>"` until exhausted or page cap.
/// 4. Group every message by `(channel_id, 6hr_bucket_start)`.
/// 5. For each closed bucket, hand off to `ingest_bucket` (same
///    canonicalise → chunk → score → seal-cascade path the periodic
///    sync uses).
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

    // 1. Channel directory — needed for canonicalisation (name +
    //    is_private flag come off `SlackChannel`).
    let channels = list_all_channels(ctx, &mut state)
        .await
        .map_err(|e| format!("[composio:slack] list_channels: {e:#}"))?;
    let channel_map: HashMap<String, SlackChannel> =
        channels.into_iter().map(|c| (c.id.clone(), c)).collect();

    // 2. User directory — for ID → display-name resolution + mention
    //    rewrites.
    let users = SlackUsers::fetch(&ctx.client).await;
    state.record_requests(1);
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
        let resp = execute_with_retry(
            &ctx.client,
            ACTION_SEARCH_MESSAGES,
            args,
            &format!("{ACTION_SEARCH_MESSAGES} page {page}"),
        )
        .await?;
        state.record_requests(1);
        dump_response("_meta", "search", page, &resp.data);

        let msgs = sync::extract_search_messages(&resp.data, &users);
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

    // 4. Group messages by (channel, 6hr-bucket-start).
    let mut by_bucket: BTreeMap<(String, chrono::DateTime<chrono::Utc>), Vec<SlackMessage>> =
        BTreeMap::new();
    for msg in all_messages {
        let bucket_start =
            crate::openhuman::memory::slack_ingestion::bucketer::bucket_start_for(msg.timestamp);
        by_bucket
            .entry((msg.channel_id.clone(), bucket_start))
            .or_default()
            .push(msg);
    }

    tracing::info!(
        connection_id = %connection_id,
        total_buckets = by_bucket.len(),
        "[composio:slack] grouped messages into buckets"
    );

    // 5. Ingest each closed bucket.
    let mut buckets_flushed = 0usize;
    let mut buckets_skipped_open = 0usize;
    let mut buckets_skipped_unknown_channel = 0usize;
    let mut buckets_failed = 0usize;

    for ((channel_id, bucket_start), mut msgs) in by_bucket {
        let Some(channel) = channel_map.get(&channel_id) else {
            buckets_skipped_unknown_channel += 1;
            continue;
        };
        msgs.sort_by_key(|m| m.timestamp);
        let bucket = crate::openhuman::memory::slack_ingestion::types::Bucket {
            start: bucket_start,
            end: bucket_end_for(bucket_start),
            messages: msgs,
        };
        // Skip still-open buckets — same closed-bucket invariant the
        // periodic sync enforces.
        if bucket.end + crate::openhuman::memory::slack_ingestion::bucketer::GRACE_PERIOD > now {
            buckets_skipped_open += 1;
            continue;
        }
        match ingest_bucket(&ctx.config, channel, &bucket, "", &connection_id).await {
            Ok(res) => {
                buckets_flushed += 1;
                tracing::info!(
                    channel = %channel.id,
                    bucket_start = %bucket.start.to_rfc3339(),
                    messages = bucket.messages.len(),
                    chunks_written = res.chunks_written,
                    "[composio:slack] ingested bucket"
                );
            }
            Err(err) => {
                buckets_failed += 1;
                tracing::warn!(
                    channel = %channel.id,
                    bucket_start = %bucket.start.to_rfc3339(),
                    error = %err,
                    "[composio:slack] ingest_bucket failed"
                );
            }
        }
    }

    let finished_at_ms = sync::now_ms();
    let summary = format!(
        "slack search-backfill: pages={page} buckets_flushed={buckets_flushed} \
         buckets_open={buckets_skipped_open} \
         unknown_channel={buckets_skipped_unknown_channel} \
         failed={buckets_failed}"
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
        items_ingested: buckets_flushed,
        started_at_ms,
        finished_at_ms,
        summary,
        details: json!({
            "pages_walked": page,
            "buckets_flushed": buckets_flushed,
            "buckets_open": buckets_skipped_open,
            "buckets_unknown_channel": buckets_skipped_unknown_channel,
            "buckets_failed": buckets_failed,
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
}
