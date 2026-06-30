//! Slack's [`IncrementalSource`] primitives.
//!
//! Slack rides the generic
//! [`crate::openhuman::memory_sync::composio::providers::orchestrator`]:
//! [`SlackProvider::sync`](super::provider::SlackProvider) delegates to
//! [`run_slack_sync`]. The orchestrator owns the control flow (budget,
//! pagination bound, dedup, the `max_items` clamp, cursor advance/hold, state
//! persistence); this module supplies only the Slack-specific shapes.
//!
//! Slack is the **structural outlier** the orchestrator's per-scope extensions
//! were built for:
//!
//!   * **Per-scope cursors** — Slack keeps one `oldest` watermark *per channel*
//!     (a `BTreeMap<channel_id, ts>` serialized into [`SyncState::cursor`])
//!     rather than a single global watermark. [`SlackSource`] opts in via
//!     [`IncrementalSource::per_scope_cursors`] and advances each channel's
//!     watermark in [`IncrementalSource::advance_scope_cursor`]. The watermark
//!     is enforced **server-side** (the `oldest` request arg), so the source
//!     also advertises [`IncrementalSource::server_side_depth`] and the
//!     orchestrator skips its client-side depth/boundary filtering.
//!   * **Per-scope error tolerance** — a single channel that errors mid-sync
//!     must not abort the other channels, so [`SlackSource`] opts into
//!     [`IncrementalSource::tolerate_scope_errors`].
//!
//! Slack dedups purely by content-hashed chunk ids (UPSERT) plus the per-channel
//! `oldest` watermark, so [`SlackSource::ingest`] returns **no** `synced_keys`
//! — the `synced_ids` set deliberately stays empty (it would otherwise grow
//! unboundedly, one entry per message, forever).
//!
//! The user directory is fetched once as a [`IncrementalSource::preamble`]
//! side-effect and stashed for every per-channel [`IncrementalSource::ingest`]
//! to resolve authors + `<@…>` mentions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::ingest::ingest_page_into_memory_tree;
use super::provider::{
    backfill_days, dump_response, execute_with_retry, list_all_channels, ACTION_FETCH_HISTORY,
    HISTORY_PAGE_SIZE, MAX_HISTORY_PAGES_PER_CHANNEL,
};
use super::sync;
use super::types::SlackChannel;
use super::users::SlackUsers;
use crate::openhuman::memory_sync::composio::providers::orchestrator::{
    self, IncrementalSource, IngestOutcome, PageFetch, SyncItem, SyncScope,
};
use crate::openhuman::memory_sync::composio::providers::sync_state::SyncState;
use crate::openhuman::memory_sync::composio::providers::{
    ProviderContext, SyncOutcome, SyncReason,
};

/// Slack's [`IncrementalSource`]. Holds the per-sync user directory and channel
/// metadata resolved in the preamble so every per-channel `fetch_page` /
/// `ingest` can read them back.
pub(crate) struct SlackSource {
    /// Workspace user directory, fetched once in [`Self::preamble`].
    users: OnceLock<SlackUsers>,
    /// Channel metadata keyed by channel id, resolved in the preamble so
    /// `ingest` can render channel labels without re-listing.
    channels: OnceLock<HashMap<String, SlackChannel>>,
    /// Wall-clock captured once per sync, used to compute the first-fetch
    /// backfill window for channels with no cursor yet.
    now: DateTime<Utc>,
    /// Monotonic counter for `dump_response` filenames (best-effort debug).
    dump_seq: AtomicU32,
}

impl SlackSource {
    fn new() -> Self {
        Self {
            users: OnceLock::new(),
            channels: OnceLock::new(),
            now: Utc::now(),
            dump_seq: AtomicU32::new(0),
        }
    }
}

/// Entry point used by [`super::provider::SlackProvider::sync`].
pub(crate) async fn run_slack_sync(
    ctx: &ProviderContext,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    orchestrator::run_sync(&SlackSource::new(), ctx, reason).await
}

#[async_trait]
impl IncrementalSource for SlackSource {
    fn toolkit(&self) -> &'static str {
        "slack"
    }

    fn page_size(&self, _reason: SyncReason) -> u32 {
        HISTORY_PAGE_SIZE
    }

    fn max_pages(&self) -> u32 {
        MAX_HISTORY_PAGES_PER_CHANNEL
    }

    fn detail_noun(&self) -> &'static str {
        "messages"
    }

    /// Slack keeps a watermark per channel, enforced server-side via `oldest`.
    fn per_scope_cursors(&self) -> bool {
        true
    }

    /// A single channel's failure must not abort the rest of the sync.
    fn tolerate_scope_errors(&self) -> bool {
        true
    }

    /// `fetch_page` injects the `oldest` window itself, so the orchestrator must
    /// not also apply a client-side depth floor (Slack `ts` isn't RFC3339).
    fn server_side_depth(&self) -> bool {
        true
    }

    /// Resolve the workspace user directory + the channel list, stash both for
    /// `ingest`, and return one [`SyncScope`] per channel.
    async fn preamble(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<Vec<SyncScope>, String> {
        // Pull the workspace user directory once per sync (soft-fails to empty).
        let (users, user_call_count) = SlackUsers::fetch(ctx).await;
        state.record_requests(user_call_count);
        tracing::info!(
            connection_id = ?ctx.connection_id,
            user_count = users.len(),
            "[composio:slack] users cached for this sync"
        );
        let _ = self.users.set(users);

        // Enumerate channels (records its own page budget).
        let channels = list_all_channels(ctx, state)
            .await
            .map_err(|e| format!("[composio:slack] list_channels: {e:#}"))?;
        tracing::info!(
            connection_id = ?ctx.connection_id,
            channel_count = channels.len(),
            "[composio:slack] channels discovered"
        );

        let scopes: Vec<SyncScope> = channels
            .iter()
            .map(|c| {
                let label = super::ingest::channel_label(&c.name, c.is_private);
                SyncScope::nested(c.id.clone(), label)
            })
            .collect();

        let channel_map: HashMap<String, SlackChannel> =
            channels.into_iter().map(|c| (c.id.clone(), c)).collect();
        let _ = self.channels.set(channel_map);

        Ok(scopes)
    }

    async fn fetch_page(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        cursor: Option<&str>,
        _reason: SyncReason,
        state: &mut SyncState,
    ) -> Result<PageFetch, String> {
        let channel_id = &scope.id;

        // Per-channel `oldest` watermark: the persisted cursor for this channel,
        // or the backfill window when the channel has never been synced. Full
        // microsecond precision is preserved so `inclusive=false` excludes only
        // the exact last-seen message. `ctx.sync_depth_days` (user-configured)
        // wins over the env-var default when set.
        let cursors = sync::decode_cursors(state.cursor.as_deref());
        let oldest_ts = cursors.get(channel_id).cloned().unwrap_or_else(|| {
            let depth_days = ctx
                .sync_depth_days
                .map(|d| d as i64)
                .unwrap_or_else(backfill_days);
            let secs = (self.now - chrono::Duration::days(depth_days)).timestamp();
            tracing::debug!(
                channel = %channel_id,
                depth_days,
                oldest_ts_secs = secs,
                "[composio:slack] [memory_sync] computing oldest_ts for backfill"
            );
            format!("{secs}.000000")
        });

        let mut args = json!({
            "channel": channel_id,
            "oldest": oldest_ts,
            "inclusive": false,
            "limit": HISTORY_PAGE_SIZE,
        });
        if let Some(c) = cursor {
            args["cursor"] = json!(c);
        }

        let (mut resp, attempts) = execute_with_retry(
            ctx,
            ACTION_FETCH_HISTORY,
            args,
            &format!("{ACTION_FETCH_HISTORY} channel={channel_id}"),
        )
        .await?;
        state.record_requests(attempts);

        let idx = self.dump_seq.fetch_add(1, Ordering::Relaxed);
        dump_response(channel_id, "history", idx, &resp.data);

        // Post-process to the slim envelope, then hand the orchestrator the raw
        // message values — `item_dedup_key` drops blanks, `ingest` enriches.
        super::post_process::post_process(ACTION_FETCH_HISTORY, None, &mut resp.data);
        let items = resp
            .data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let next = sync::extract_next_cursor(&resp.data);

        tracing::debug!(
            channel = %channel_id,
            fetched = items.len(),
            has_next = next.is_some(),
            "[composio:slack] history page"
        );

        Ok(PageFetch { items, next })
    }

    /// Dedup key doubles as the validity gate: a message with no parseable `ts`
    /// or blank text returns `None` so the orchestrator drops it **before** the
    /// `max_items` clamp — keeping the cap counted against *valid* messages,
    /// exactly as the old per-channel loop did (it filtered then clamped).
    fn item_dedup_key(&self, item: &Value) -> Option<String> {
        let ts = item.get("ts").and_then(Value::as_str)?;
        sync::parse_ts(ts)?;
        let text = item.get("text").and_then(Value::as_str).unwrap_or("");
        if text.trim().is_empty() {
            return None;
        }
        Some(ts.to_string())
    }

    fn item_sort_ts(&self, item: &Value) -> Option<String> {
        item.get("ts").and_then(Value::as_str).map(str::to_string)
    }

    /// Advance this channel's watermark inside the per-channel cursor map
    /// serialized in [`SyncState::cursor`].
    fn advance_scope_cursor(&self, state: &mut SyncState, scope: &SyncScope, newest_ts: &str) {
        let mut cursors = sync::decode_cursors(state.cursor.as_deref());
        cursors.insert(scope.id.clone(), newest_ts.to_string());
        state.cursor = Some(sync::encode_cursors(&cursors));
    }

    async fn ingest(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        _state: &mut SyncState,
        items: Vec<SyncItem>,
    ) -> IngestOutcome {
        let channel_id = &scope.id;
        // Channel metadata from the preamble (falls back to a bare id-named
        // channel if the map is somehow missing this scope).
        let channel = self
            .channels
            .get()
            .and_then(|m| m.get(channel_id))
            .cloned()
            .unwrap_or_else(|| SlackChannel {
                id: channel_id.clone(),
                name: channel_id.clone(),
                is_private: false,
            });
        let users = self.users.get().cloned().unwrap_or_else(SlackUsers::empty);

        // Enrich the surviving raw message values into canonical SlackMessages
        // (author resolution + `<@…>` mention rewriting) via the shared parser.
        let raws: Vec<Value> = items.into_iter().map(|it| it.raw).collect();
        let wrapped = json!({ "messages": raws });
        let messages = sync::extract_messages(&wrapped, &channel, &users);

        if messages.is_empty() {
            return IngestOutcome::default();
        }

        let connection_id = ctx.connection_id.as_deref().unwrap_or("default");
        let count = messages.len();
        match ingest_page_into_memory_tree(&ctx.config, "", connection_id, &messages).await {
            Ok(chunks) => {
                tracing::info!(
                    channel = %channel_id,
                    messages = count,
                    chunks,
                    "[composio:slack] channel ingest done"
                );
                IngestOutcome {
                    // No synced_keys: Slack dedups via content-hash UPSERT + the
                    // per-channel watermark, so `synced_ids` stays empty.
                    synced_keys: Vec::new(),
                    persisted: count,
                    had_failures: false,
                }
            }
            Err(e) => {
                tracing::warn!(
                    channel = %channel_id,
                    error = %e,
                    "[composio:slack] ingest_page_into_memory_tree failed (watermark held)"
                );
                IngestOutcome {
                    synced_keys: Vec::new(),
                    persisted: 0,
                    had_failures: true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source() -> SlackSource {
        SlackSource::new()
    }

    #[test]
    fn item_dedup_key_drops_blank_and_unparseable() {
        let s = source();
        // Valid message → keyed by ts.
        assert_eq!(
            s.item_dedup_key(&json!({ "ts": "1714003200.000100", "text": "hi" }))
                .as_deref(),
            Some("1714003200.000100")
        );
        // Blank text → dropped (None), so it never counts against the cap.
        assert_eq!(
            s.item_dedup_key(&json!({ "ts": "1714003200.000100", "text": "   " })),
            None
        );
        // Bot-authored (no `user`) but non-blank text → kept (parity with the
        // old extractor, which retained author-less messages).
        assert_eq!(
            s.item_dedup_key(&json!({ "ts": "1714003300.000200", "text": "bot update" }))
                .as_deref(),
            Some("1714003300.000200")
        );
        // Unparseable ts → dropped.
        assert_eq!(
            s.item_dedup_key(&json!({ "ts": "nope", "text": "hi" })),
            None
        );
        // Missing ts → dropped.
        assert_eq!(s.item_dedup_key(&json!({ "text": "hi" })), None);
    }

    #[test]
    fn item_sort_ts_reads_raw_ts() {
        let s = source();
        assert_eq!(
            s.item_sort_ts(&json!({ "ts": "1714003200.000100" }))
                .as_deref(),
            Some("1714003200.000100")
        );
        assert_eq!(s.item_sort_ts(&json!({ "text": "no ts" })), None);
    }

    #[test]
    fn advance_scope_cursor_merges_into_per_channel_map() {
        let s = source();
        let mut state = SyncState::new("slack", "conn1");
        state.cursor = Some(r#"{"C1":"1714003200.000100"}"#.to_string());

        s.advance_scope_cursor(
            &mut state,
            &SyncScope::nested("C2", "#two"),
            "1714010000.000200",
        );
        let map = sync::decode_cursors(state.cursor.as_deref());
        assert_eq!(map.get("C1").map(String::as_str), Some("1714003200.000100"));
        assert_eq!(map.get("C2").map(String::as_str), Some("1714010000.000200"));

        // Re-advancing an existing channel overwrites just that entry.
        s.advance_scope_cursor(
            &mut state,
            &SyncScope::nested("C1", "#one"),
            "1714099999.000300",
        );
        let map = sync::decode_cursors(state.cursor.as_deref());
        assert_eq!(map.get("C1").map(String::as_str), Some("1714099999.000300"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn source_advertises_slack_outlier_hooks() {
        let s = source();
        assert!(s.per_scope_cursors());
        assert!(s.tolerate_scope_errors());
        assert!(s.server_side_depth());
        assert_eq!(s.toolkit(), "slack");
        assert_eq!(s.detail_noun(), "messages");
        assert_eq!(s.page_size(SyncReason::Periodic), HISTORY_PAGE_SIZE);
        assert_eq!(s.max_pages(), MAX_HISTORY_PAGES_PER_CHANNEL);
    }
}
