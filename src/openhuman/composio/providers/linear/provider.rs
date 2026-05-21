//! Linear provider — incremental sync of issues assigned to the
//! authenticated user, with per-item persistence into the Memory Tree.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Resolve the authenticated viewer's id via `LINEAR_GET_VIEWER`
//!      (Linear's "current user" probe); re-check the budget after
//!      the probe so the next call honours a tight cap strictly.
//!   4. Page through `LINEAR_LIST_LINEAR_ISSUES` filtered to the user
//!      as `assignee`, sorted by `updatedAt` descending. Stop early
//!      once issues older than the cursor are reached, or when
//!      `pageInfo.hasNextPage == false` signals end-of-results.
//!   5. For each issue, persist as a single memory document if it's
//!      new *or* edited since the last sync.
//!   6. Advance the cursor to the newest `updatedAt` seen and save.
//!
//! Privacy posture: only issues the user is assigned to are pulled,
//! never the whole workspace's issue graph. Mirrors the
//! "fetch-what-the-user-sees" model `gmail` / `notion` / `clickup`
//! already follow.

use async_trait::async_trait;
use serde_json::json;

use super::sync;
use crate::openhuman::composio::providers::sync_state::{persist_single_item, SyncState};
use crate::openhuman::composio::providers::{
    pick_str, ComposioProvider, CuratedTool, ProviderContext, ProviderUserProfile, SyncOutcome,
    SyncReason,
};

pub(crate) const ACTION_GET_VIEWER: &str = "LINEAR_GET_VIEWER";
pub(crate) const ACTION_LIST_ISSUES: &str = "LINEAR_LIST_LINEAR_ISSUES";

/// Page size per API call. Linear's `issues` connection accepts up to
/// 250, but we stick with a smaller window on steady-state syncs to
/// keep response sizes bounded and pagination cheap to back-off.
const PAGE_SIZE: u32 = 50;

/// Larger initial-sync page size, used immediately after OAuth so the
/// first backfill catches up faster.
const INITIAL_PAGE_SIZE: u32 = 100;

/// Maximum pages per sync pass before yielding. Caps initial backfill
/// churn — anything beyond this rolls over to the next sync interval.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Paths for extracting an issue's unique id (UUID). Composio sometimes
/// wraps the upstream payload under `data`, so we check both shapes.
const ISSUE_ID_PATHS: &[&str] = &["id", "data.id"];

pub struct LinearProvider;

impl LinearProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LinearProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for LinearProvider {
    fn toolkit_slug(&self) -> &'static str {
        "linear"
    }

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(super::tools::LINEAR_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        // 30 minutes — same cadence as Notion and ClickUp. Linear
        // issues change at a similar rate to PM tooling, so this is in
        // the right ballpark.
        Some(30 * 60)
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:linear] fetch_user_profile via {ACTION_GET_VIEWER}"
        );

        let resp = ctx
            .execute(ACTION_GET_VIEWER, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_GET_VIEWER} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:linear] {ACTION_GET_VIEWER}: {err}"));
        }

        // Composio's wrapping puts Linear's `{ viewer: {...} }` GraphQL
        // payload at `data.viewer` (or `viewer` un-wrapped). `pick_str`
        // walks dotted paths so both forms work.
        let data = &resp.data;
        let display_name = pick_str(
            data,
            &[
                "viewer.displayName",
                "data.viewer.displayName",
                "viewer.name",
                "data.viewer.name",
            ],
        );
        let email = pick_str(data, &["viewer.email", "data.viewer.email"]);
        let username = sync::extract_viewer_id(data);
        let avatar_url = pick_str(data, &["viewer.avatarUrl", "data.viewer.avatarUrl"]);
        let profile_url = pick_str(data, &["viewer.url", "data.viewer.url"]);

        Ok(ProviderUserProfile {
            toolkit: "linear".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email,
            username,
            avatar_url,
            profile_url,
            extras: data.clone(),
        })
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
            "[composio:linear] incremental sync starting"
        );

        // ── Step 1: load persistent sync state ──────────────────────
        let Some(memory) = ctx.memory_client() else {
            return Err("[composio:linear] memory client not ready".to_string());
        };
        let mut state = SyncState::load(&memory, "linear", &connection_id).await?;

        // ── Step 2: check daily budget ──────────────────────────────
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:linear] daily request budget exhausted, skipping sync"
            );
            return Ok(SyncOutcome {
                toolkit: "linear".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "linear sync skipped: daily budget exhausted".to_string(),
                details: json!({ "budget_exhausted": true }),
            });
        }

        // ── Step 3: resolve the authenticated viewer's id ───────────
        //
        // Linear's "list issues" endpoint accepts an `assignee` filter
        // keyed on the user's id. We need the *current* user's id to
        // scope the sync to "my issues" rather than "everyone's
        // issues". The id is stable for the lifetime of the OAuth
        // connection, but we re-fetch on every sync rather than
        // persisting it because the call is cheap and re-fetching
        // implicitly validates the OAuth connection is still good
        // before we start paginating.
        let viewer_id = match self.resolve_viewer_id(ctx, &mut state).await {
            Ok(id) => id,
            Err(e) => {
                let _ = state.save(&memory).await;
                return Err(e);
            }
        };

        // Re-check the budget here — `resolve_viewer_id` just spent
        // one request, and if that pushed us over the cap, firing
        // `LINEAR_LIST_LINEAR_ISSUES` would be wasted work. Same
        // discipline as `clickup::ClickUpProvider::sync` between the
        // user-id probe and the workspace lookup (per the
        // CodeRabbit feedback on #2291).
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:linear] budget exhausted after viewer probe, skipping sync"
            );
            state.save(&memory).await?;
            return Ok(SyncOutcome {
                toolkit: "linear".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "linear sync skipped: daily budget exhausted after viewer probe"
                    .to_string(),
                details: json!({ "budget_exhausted": true, "viewer_id_resolved": true }),
            });
        }

        // ── Step 4: paginated incremental fetch ─────────────────────
        let page_size = match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        };

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut newest_updated: Option<String> = None;
        let mut next_cursor: Option<String> = None;

        for page_num in 0..MAX_PAGES_PER_SYNC {
            if state.budget_exhausted() {
                tracing::info!(
                    connection_id = %connection_id,
                    page = page_num,
                    "[composio:linear] budget exhausted mid-sync, stopping pagination"
                );
                break;
            }

            let mut args = json!({
                "first": page_size,
                "filter": {
                    "assignee": { "id": { "eq": viewer_id.clone() } }
                },
                // Linear's `orderBy` for the `issues` connection accepts
                // `updatedAt`. Descending ensures the freshest items
                // arrive first so the cursor advances quickly.
                "orderBy": "updatedAt",
                "sortDirection": "descending"
            });
            if let Some(ref cursor) = next_cursor {
                args["after"] = json!(cursor);
            }

            // Transport-level failure (network, timeout, deserialise error)
            // must persist the request counters / synced markers accumulated
            // earlier in this sync pass — otherwise a flap in the middle of
            // pagination silently rolls back budget accounting and the next
            // sync would burn through the daily cap re-fetching pages we
            // already drained. The successful-response branch below already
            // does this (line "let _ = state.save(&memory).await" before the
            // Err return); this match mirrors that discipline at the
            // `ctx.execute` boundary too.
            let resp = match ctx.execute(ACTION_LIST_ISSUES, Some(args)).await {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = state.save(&memory).await;
                    return Err(format!(
                        "[composio:linear] {ACTION_LIST_ISSUES} page {page_num}: {e:#}"
                    ));
                }
            };

            state.record_requests(1);

            if !resp.successful {
                let err = resp
                    .error
                    .clone()
                    .unwrap_or_else(|| "provider reported failure".to_string());
                let _ = state.save(&memory).await;
                return Err(format!(
                    "[composio:linear] {ACTION_LIST_ISSUES} page {page_num}: {err}"
                ));
            }

            let issues = sync::extract_issues(&resp.data);
            total_fetched += issues.len();

            if issues.is_empty() {
                tracing::debug!(
                    connection_id = %connection_id,
                    page = page_num,
                    "[composio:linear] empty page, stopping pagination"
                );
                break;
            }

            // ── Per-item dedup + persist ───────────────────────────
            let mut hit_cursor_boundary = false;
            for issue in &issues {
                let Some(issue_id) =
                    crate::openhuman::composio::providers::sync_state::extract_item_id(
                        issue,
                        ISSUE_ID_PATHS,
                    )
                else {
                    tracing::debug!(
                        connection_id = %connection_id,
                        "[composio:linear] issue missing id, skipping"
                    );
                    continue;
                };

                let updated = sync::extract_issue_updated(issue);

                // Track newest `updatedAt` for cursor advancement.
                if let Some(ref ts) = updated {
                    if newest_updated.as_ref().is_none_or(|existing| ts > existing) {
                        newest_updated = Some(ts.clone());
                    }
                }

                // Composite (issue_id, updatedAt) sync key — re-syncs
                // edited issues. Same trick as Notion's
                // `last_edited_time` and ClickUp's `date_updated`.
                let sync_key = match &updated {
                    Some(ts) => format!("{issue_id}@{ts}"),
                    None => issue_id.clone(),
                };

                // If `updatedAt` is at or older than our cursor *and*
                // we've already synced this composite key, the rest of
                // the page is by definition older too — stop early.
                if let (Some(ref cursor), Some(ref ts)) = (&state.cursor, &updated) {
                    if ts <= cursor && state.is_synced(&sync_key) {
                        hit_cursor_boundary = true;
                        continue;
                    }
                }

                if state.is_synced(&sync_key) {
                    continue;
                }

                let title_text = sync::extract_issue_title(issue)
                    .unwrap_or_else(|| format!("Linear issue {issue_id}"));
                let identifier_hint = sync::extract_issue_identifier(issue);
                let doc_id = format!("composio-linear-issue-{issue_id}");
                // Surface the workspace-prefixed identifier (e.g.
                // `OH-123`) in the title when available — it's how
                // humans refer to Linear issues in conversation.
                let title = match identifier_hint {
                    Some(ident) => format!("Linear {ident}: {title_text}"),
                    None => format!("Linear: {title_text}"),
                };

                match persist_single_item(
                    &memory,
                    "linear",
                    &doc_id,
                    &title,
                    issue,
                    "linear",
                    ctx.connection_id.as_deref(),
                )
                .await
                {
                    Ok(_) => {
                        state.mark_synced(&sync_key);
                        total_persisted += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            connection_id = %connection_id,
                            issue_id = %issue_id,
                            error = %e,
                            "[composio:linear] failed to persist issue (continuing)"
                        );
                    }
                }
            }

            if hit_cursor_boundary {
                tracing::debug!(
                    connection_id = %connection_id,
                    page = page_num,
                    "[composio:linear] reached cursor boundary, stopping pagination"
                );
                break;
            }

            // Linear's Relay-style pagination: only continue while
            // `pageInfo.hasNextPage` is true. `extract_pagination_end_cursor`
            // returns `None` when there's no next page.
            next_cursor = sync::extract_pagination_end_cursor(&resp.data);
            if next_cursor.is_none() {
                tracing::debug!(
                    connection_id = %connection_id,
                    page = page_num,
                    "[composio:linear] no next cursor, done"
                );
                break;
            }
        }

        // ── Step 5: advance cursor and save state ───────────────────
        if let Some(new_cursor) = newest_updated {
            state.advance_cursor(&new_cursor);
        }
        state.set_last_sync_at_ms(sync::now_ms());
        state.save(&memory).await?;

        let finished_at_ms = sync::now_ms();
        let summary = format!(
            "linear sync ({reason}): fetched {total_fetched}, persisted {total_persisted} new, \
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
            "[composio:linear] incremental sync complete"
        );

        Ok(SyncOutcome {
            toolkit: "linear".to_string(),
            connection_id: Some(connection_id),
            reason: reason.as_str().to_string(),
            items_ingested: total_persisted,
            started_at_ms,
            finished_at_ms,
            summary,
            details: json!({
                "issues_fetched": total_fetched,
                "issues_persisted": total_persisted,
                "budget_remaining": state.budget_remaining(),
                "cursor": state.cursor,
                "synced_ids_total": state.synced_ids.len(),
            }),
        })
    }
}

impl LinearProvider {
    /// Look up (and budget-record) the authenticated viewer's id.
    ///
    /// The id is stable for the connection's lifetime, but we re-fetch
    /// on every sync rather than persisting it because (a) the call is
    /// cheap, (b) caching it in `SyncState` would inflate the public
    /// struct for a single provider's quirk, and (c) it implicitly
    /// validates the OAuth connection is still good before paginating.
    async fn resolve_viewer_id(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<String, String> {
        let resp = ctx
            .execute(ACTION_GET_VIEWER, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_GET_VIEWER} failed: {e:#}"))?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:linear] {ACTION_GET_VIEWER}: {err}"));
        }

        sync::extract_viewer_id(&resp.data)
            .ok_or_else(|| "[composio:linear] LINEAR_GET_VIEWER returned no viewer.id".to_string())
    }
}
