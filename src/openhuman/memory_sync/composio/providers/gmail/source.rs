//! Gmail's [`IncrementalSource`] primitives.
//!
//! Gmail rides the generic
//! [`crate::openhuman::memory_sync::composio::providers::orchestrator`]:
//! [`GmailProvider::sync`](super::provider::GmailProvider) delegates to
//! [`run_gmail_sync`]. The orchestrator owns the control flow (budget,
//! pagination bound, dedup, the precise `max_items` clamp, cursor advance/hold,
//! state persistence); this module supplies only the Gmail-specific shapes.
//!
//! Gmail is **flat**, with three quirks handled by trait hooks:
//!   * **server-side depth** — the `sync_depth_days` window is injected as an
//!     `after:<epoch>` query qualifier on the first sync (no cursor), so it
//!     overrides [`IncrementalSource::server_side_depth`].
//!   * **adaptive page ceiling** — when the previous sync ran within the last
//!     few minutes, [`GmailSource::page_ceiling`] caps pagination aggressively.
//!   * **stop on an all-synced page** — Gmail's result set is newest-first, so a
//!     page whose messages are all already-synced means nothing newer is left
//!     ([`IncrementalSource::stop_on_empty_pending`] = `true`).
//!
//! The account email (used as the per-account ingest `source_id`) is resolved
//! once in the preamble — **not** budget-counted, matching the original — and
//! stashed on the source so [`GmailSource::ingest`] can read it back. Messages
//! ingest as a single batch per page; dedup is keyed by the bare message id
//! (emails are immutable).
//!
//! ## One intentional behavior drop
//!
//! The legacy loop also had a `last_seen_id` "head-unchanged" micro-optimization
//! (stop before post-processing page 0 when its first id matches the last sync's
//! freshest id). That is **redundant** with `stop_on_empty_pending`: on a quiet
//! inbox both fetch exactly one page and stop, so the API-budget behavior is
//! unchanged. It is dropped here (and `last_seen_id` is no longer written) to
//! avoid a gmail-only orchestrator hook for a no-op-on-budget optimization.

use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::ingest::ingest_page_into_memory_tree_with_outcome;
use super::provider::{ACTION_FETCH_EMAILS, ACTION_GET_PROFILE, BASE_QUERY};
use super::sync;
use crate::openhuman::memory_sync::composio::providers::orchestrator::{
    self, IncrementalSource, IngestOutcome, PageFetch, SyncItem, SyncScope,
};
use crate::openhuman::memory_sync::composio::providers::sync_state::{extract_item_id, SyncState};
use crate::openhuman::memory_sync::composio::providers::{
    pick_str, ProviderContext, SyncOutcome, SyncReason,
};

/// Page size per API call.
const PAGE_SIZE: u32 = 25;

/// Larger page size for the very first sync after OAuth.
const INITIAL_PAGE_SIZE: u32 = 50;

/// Maximum pages to fetch in a single sync pass.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Adaptive page cap applied when a successful sync ran very recently.
const RECENT_SYNC_MAX_PAGES: u32 = 2;

/// "Recent" window (ms) used by the adaptive page cap.
const RECENT_SYNC_WINDOW_MS: u64 = 5 * 60 * 1000;

/// Paths to try when extracting a message's unique id.
const MESSAGE_ID_PATHS: &[&str] = &["id", "data.id", "messageId", "data.messageId"];

/// Paths for extracting the internal date (epoch millis or date string) used as
/// the sync cursor.
const MESSAGE_DATE_PATHS: &[&str] = &[
    "internalDate",
    "data.internalDate",
    "date",
    "data.date",
    "receivedAt",
    "data.receivedAt",
];

/// Gmail's [`IncrementalSource`]. Holds the account email resolved in the
/// preamble so [`Self::ingest`] can stamp a stable per-account `source_id`.
pub(crate) struct GmailSource {
    account_email: OnceLock<Option<String>>,
}

impl GmailSource {
    fn new() -> Self {
        Self {
            account_email: OnceLock::new(),
        }
    }

    /// Resolve the account email via `GMAIL_GET_PROFILE`. Tolerant of failure
    /// (returns `None` → ingest falls back to per-participants bucketing) and
    /// **not** budget-counted, matching the original sync.
    async fn resolve_account_email(&self, ctx: &ProviderContext) -> Option<String> {
        match ctx.execute(ACTION_GET_PROFILE, Some(json!({}))).await {
            Ok(resp) if resp.successful => pick_str(
                &resp.data,
                &["emailAddress", "email", "profile.emailAddress"],
            ),
            Ok(_) | Err(_) => None,
        }
    }
}

/// Entry point used by [`super::provider::GmailProvider::sync`]. Bumps the
/// in-process scheduler heartbeat on completion (parity with the original),
/// so a periodic tick doesn't immediately re-fire on top of a trigger- or
/// connection-created sync.
pub(crate) async fn run_gmail_sync(
    ctx: &ProviderContext,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    let connection_id = ctx
        .connection_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let outcome = orchestrator::run_sync(&GmailSource::new(), ctx, reason).await?;
    crate::openhuman::memory_sync::composio::periodic::record_sync_success("gmail", &connection_id);
    Ok(outcome)
}

#[async_trait]
impl IncrementalSource for GmailSource {
    fn toolkit(&self) -> &'static str {
        "gmail"
    }

    fn page_size(&self, reason: SyncReason) -> u32 {
        match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        }
    }

    fn max_pages(&self) -> u32 {
        MAX_PAGES_PER_SYNC
    }

    /// Adaptive cap: an initial backfill gets the full ceiling; a steady-state
    /// sync that ran within [`RECENT_SYNC_WINDOW_MS`] is capped aggressively
    /// (recent ticks rarely need more than a couple of pages).
    fn page_ceiling(&self, reason: SyncReason, state: &SyncState) -> u32 {
        match reason {
            SyncReason::ConnectionCreated => MAX_PAGES_PER_SYNC,
            _ => match state.last_sync_at_ms {
                Some(last_ms) if sync::now_ms().saturating_sub(last_ms) < RECENT_SYNC_WINDOW_MS => {
                    RECENT_SYNC_MAX_PAGES
                }
                _ => MAX_PAGES_PER_SYNC,
            },
        }
    }

    fn stop_on_empty_pending(&self) -> bool {
        true
    }

    fn server_side_depth(&self) -> bool {
        true
    }

    fn detail_noun(&self) -> &'static str {
        "messages"
    }

    /// Resolve the account email (stashed for `ingest`) and return the single
    /// flat scope. The profile fetch is not budget-counted (parity).
    async fn preamble(
        &self,
        ctx: &ProviderContext,
        _state: &mut SyncState,
    ) -> Result<Vec<SyncScope>, String> {
        let email = self.resolve_account_email(ctx).await;
        if email.is_none() {
            tracing::warn!(
                connection_id = ?ctx.connection_id,
                "[composio:gmail] account email unresolved; ingest falls back to per-participants source_id"
            );
        }
        let _ = self.account_email.set(email);
        Ok(vec![SyncScope::flat()])
    }

    async fn fetch_page(
        &self,
        ctx: &ProviderContext,
        _scope: &SyncScope,
        cursor: Option<&str>,
        reason: SyncReason,
        state: &mut SyncState,
    ) -> Result<PageFetch, String> {
        // Build the Gmail search query. Prefer a second-precision
        // `after:<unix>` from the persistent cursor; on the first sync inject
        // the `sync_depth_days` floor (server-side depth).
        let mut query = BASE_QUERY.to_string();
        if let Some(persistent) = state.cursor.as_deref() {
            if let Some(epoch_filter) = sync::cursor_to_gmail_after_epoch_filter(persistent) {
                query.push_str(&format!(" after:{epoch_filter}"));
            } else if let Some(date_filter) = sync::cursor_to_gmail_after_filter(persistent) {
                query.push_str(&format!(" after:{date_filter}"));
            }
        } else if let Some(days) = ctx.sync_depth_days {
            let floor_secs = super::super::helpers::epoch_floor_from_depth(days);
            query.push_str(&format!(" after:{floor_secs}"));
        }

        let mut args = json!({
            "max_results": self.page_size(reason),
            "query": query,
        });
        // The orchestrator's opaque cursor carries Gmail's page token.
        if let Some(token) = cursor {
            args["page_token"] = json!(token);
        }

        let mut resp = ctx
            .execute(ACTION_FETCH_EMAILS, Some(args.clone()))
            .await
            .map_err(|e| format!("[composio:gmail] {ACTION_FETCH_EMAILS}: {e:#}"))?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:gmail] {ACTION_FETCH_EMAILS}: {err}"));
        }

        // Pull the backend's pre-rendered `markdownFormatted` onto each message
        // before post-process, then slim/normalize the envelope (same order as
        // the original sync).
        if let Some(top_md) = resp
            .markdown_formatted
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            super::post_process::apply_response_level_markdown(&mut resp.data, top_md);
        }
        super::post_process::post_process(ACTION_FETCH_EMAILS, Some(&args), &mut resp.data);

        Ok(PageFetch {
            items: sync::extract_messages(&resp.data),
            next: sync::extract_page_token(&resp.data),
        })
    }

    /// Emails are immutable — dedup by the bare message id (no `@version`).
    fn item_dedup_key(&self, item: &Value) -> Option<String> {
        extract_item_id(item, MESSAGE_ID_PATHS)
    }

    fn item_sort_ts(&self, item: &Value) -> Option<String> {
        extract_item_id(item, MESSAGE_DATE_PATHS)
    }

    /// Batch-ingest the page's new messages into the memory tree. `synced_keys`
    /// are the message ids that actually ingested; a partial ingest trips
    /// `had_failures` so the orchestrator holds the cursor for retry.
    async fn ingest(
        &self,
        ctx: &ProviderContext,
        _scope: &SyncScope,
        _state: &mut SyncState,
        items: Vec<SyncItem>,
    ) -> IngestOutcome {
        if items.is_empty() {
            return IngestOutcome::default();
        }
        let connection_id = ctx.connection_id.as_deref().unwrap_or("default");
        let owner = format!("gmail-sync:{connection_id}");
        let account_email = self.account_email.get().cloned().flatten();
        let total = items.len();
        let messages: Vec<Value> = items.into_iter().map(|it| it.raw).collect();

        match ingest_page_into_memory_tree_with_outcome(
            ctx.config.as_ref(),
            &owner,
            account_email.as_deref(),
            &messages,
        )
        .await
        {
            Ok(outcome) => {
                let persisted = outcome.item_ids_ingested.len();
                IngestOutcome {
                    synced_keys: outcome.item_ids_ingested,
                    persisted,
                    // A short ingest (fewer messages persisted than handed in)
                    // holds the cursor so the next sync re-scans the gap.
                    had_failures: persisted < total,
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %format!("{e:#}"),
                    new_messages = total,
                    "[composio:gmail] ingest_page_into_memory_tree failed (continuing)"
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

    #[test]
    fn item_dedup_key_is_bare_message_id() {
        // No `@version` — emails are immutable.
        assert_eq!(
            GmailSource::new()
                .item_dedup_key(&json!({ "messageId": "m1", "internalDate": "1780000000000" }))
                .as_deref(),
            Some("m1")
        );
        assert_eq!(
            GmailSource::new().item_dedup_key(&json!({ "internalDate": "x" })),
            None
        );
    }

    #[test]
    fn gmail_advertises_server_side_depth_and_empty_stop() {
        let s = GmailSource::new();
        assert!(s.server_side_depth());
        assert!(s.stop_on_empty_pending());
        assert_eq!(s.detail_noun(), "messages");
    }

    #[test]
    fn page_ceiling_caps_aggressively_after_a_recent_sync() {
        let s = GmailSource::new();
        let mut state = SyncState::new("gmail", "c");
        // Initial backfill always gets the full ceiling.
        assert_eq!(
            s.page_ceiling(SyncReason::ConnectionCreated, &state),
            MAX_PAGES_PER_SYNC
        );
        // A steady-state sync right after the previous one is capped.
        state.set_last_sync_at_ms(sync::now_ms());
        assert_eq!(
            s.page_ceiling(SyncReason::Periodic, &state),
            RECENT_SYNC_MAX_PAGES
        );
        // A stale last-sync drops back to the full ceiling.
        state.set_last_sync_at_ms(0);
        assert_eq!(
            s.page_ceiling(SyncReason::Periodic, &state),
            MAX_PAGES_PER_SYNC
        );
    }
}
