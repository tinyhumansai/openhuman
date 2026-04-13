//! Notion provider — incremental sync with per-item persistence.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Fetch a page of recently edited pages via `NOTION_FETCH_DATA`,
//!      sorted by `last_edited_time` descending. When a cursor exists
//!      we can stop as soon as we see pages older than the cursor.
//!   4. Deduplicate against `synced_ids` in the state. Pages that have
//!      been *edited* since their last sync are re-persisted (the cursor
//!      is based on `last_edited_time`, so an edited page appears again).
//!   5. Persist each **new or updated** page as its own memory document.
//!   6. Paginate (up to budget) until no more results or all items in the
//!      page are older than the cursor.
//!   7. Advance the cursor and save state.

mod sync;
#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::sync_state::{extract_item_id, persist_single_item, SyncState};
use super::{
    pick_str, ComposioProvider, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};

pub(crate) const ACTION_GET_ABOUT_ME: &str = "NOTION_GET_ABOUT_ME";
pub(crate) const ACTION_FETCH_DATA: &str = "NOTION_FETCH_DATA";

/// Page size per API call.
const PAGE_SIZE: u32 = 25;

/// Larger page size for initial sync after OAuth.
const INITIAL_PAGE_SIZE: u32 = 50;

/// Maximum pages per sync pass.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Paths for extracting a page's unique ID.
const PAGE_ID_PATHS: &[&str] = &["id", "data.id", "pageId", "data.pageId"];

/// Paths for extracting the `last_edited_time` used as sync cursor.
const PAGE_EDITED_PATHS: &[&str] = &[
    "last_edited_time",
    "data.last_edited_time",
    "lastEditedTime",
    "data.lastEditedTime",
];

pub struct NotionProvider;

impl NotionProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NotionProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for NotionProvider {
    fn toolkit_slug(&self) -> &'static str {
        "notion"
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        Some(30 * 60)
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:notion] fetch_user_profile via {ACTION_GET_ABOUT_ME}"
        );

        let resp = ctx
            .client
            .execute_tool(ACTION_GET_ABOUT_ME, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:notion] {ACTION_GET_ABOUT_ME} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:notion] {ACTION_GET_ABOUT_ME}: {err}"));
        }

        let data = &resp.data;
        let display_name = pick_str(
            data,
            &[
                "data.name",
                "data.user.name",
                "name",
                "data.bot.owner.user.name",
            ],
        );
        let email = pick_str(
            data,
            &[
                "data.person.email",
                "data.user.person.email",
                "person.email",
                "email",
            ],
        );
        let username = pick_str(
            data,
            &["data.bot.owner.user.id", "data.id", "id", "data.user.id"],
        );
        let avatar_url = pick_str(
            data,
            &["data.avatar_url", "data.user.avatar_url", "avatar_url"],
        );

        Ok(ProviderUserProfile {
            toolkit: "notion".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email,
            username,
            avatar_url,
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
            "[composio:notion] incremental sync starting"
        );

        // ── Step 1: load persistent sync state ──────────────────────
        let Some(memory) = ctx.memory_client() else {
            return Err("[composio:notion] memory client not ready".to_string());
        };
        let mut state = SyncState::load(&memory, "notion", &connection_id).await?;

        // ── Step 2: check daily budget ──────────────────────────────
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:notion] daily request budget exhausted, skipping sync"
            );
            return Ok(SyncOutcome {
                toolkit: "notion".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "notion sync skipped: daily budget exhausted".to_string(),
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
        let mut newest_edited_time: Option<String> = None;
        let mut notion_cursor: Option<String> = None;

        for page_num in 0..MAX_PAGES_PER_SYNC {
            if state.budget_exhausted() {
                tracing::info!(
                    page = page_num,
                    "[composio:notion] budget exhausted mid-sync, stopping pagination"
                );
                break;
            }

            let mut args = json!({
                "page_size": page_size,
                "filter": { "value": "page", "property": "object" },
                "sort": { "direction": "descending", "timestamp": "last_edited_time" }
            });
            if let Some(ref cursor) = notion_cursor {
                args["start_cursor"] = json!(cursor);
            }

            let resp = ctx
                .client
                .execute_tool(ACTION_FETCH_DATA, Some(args))
                .await
                .map_err(|e| {
                    format!("[composio:notion] {ACTION_FETCH_DATA} page {page_num}: {e:#}")
                })?;

            state.record_requests(1);

            if !resp.successful {
                let err = resp
                    .error
                    .clone()
                    .unwrap_or_else(|| "provider reported failure".to_string());
                let _ = state.save(&memory).await;
                return Err(format!(
                    "[composio:notion] {ACTION_FETCH_DATA} page {page_num}: {err}"
                ));
            }

            let results = sync::extract_results(&resp.data);
            total_fetched += results.len();

            if results.is_empty() {
                tracing::debug!(
                    page = page_num,
                    "[composio:notion] empty page, stopping pagination"
                );
                break;
            }

            // ── Step 4: deduplicate and persist per-item ────────────
            let mut hit_cursor_boundary = false;
            for page in &results {
                let Some(page_id) = extract_item_id(page, PAGE_ID_PATHS) else {
                    tracing::debug!("[composio:notion] page missing ID, skipping");
                    continue;
                };

                let edited_time = extract_item_id(page, PAGE_EDITED_PATHS);

                // Track the newest edited time for cursor advancement.
                if let Some(ref et) = edited_time {
                    if newest_edited_time
                        .as_ref()
                        .map_or(true, |existing| et > existing)
                    {
                        newest_edited_time = Some(et.clone());
                    }
                }

                // For Notion, a page can be *edited* after we last synced
                // it. We use a composite key of page_id + edited_time to
                // detect this: if the page_id is in synced_ids but the
                // edited_time is newer than the cursor, we re-sync it.
                let sync_key = match &edited_time {
                    Some(et) => format!("{page_id}@{et}"),
                    None => page_id.clone(),
                };

                // If the page's edited time is older than our cursor,
                // we've caught up — everything beyond is already synced.
                if let (Some(ref cursor), Some(ref et)) = (&state.cursor, &edited_time) {
                    if et <= cursor && state.is_synced(&sync_key) {
                        hit_cursor_boundary = true;
                        continue;
                    }
                }

                if state.is_synced(&sync_key) {
                    continue;
                }

                // Build a title from the page's properties.
                let title_text = sync::extract_page_title(page)
                    .unwrap_or_else(|| format!("Notion page {page_id}"));
                let doc_id = format!("composio-notion-page-{page_id}");
                let title = format!("Notion: {title_text}");

                match persist_single_item(
                    &memory,
                    "notion",
                    &doc_id,
                    &title,
                    page,
                    "notion",
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
                            page_id = %page_id,
                            error = %e,
                            "[composio:notion] failed to persist page (continuing)"
                        );
                    }
                }
            }

            if hit_cursor_boundary {
                tracing::debug!(
                    page = page_num,
                    "[composio:notion] reached cursor boundary, stopping"
                );
                break;
            }

            // Check for next page cursor from Notion API.
            notion_cursor = sync::extract_notion_cursor(&resp.data);
            if notion_cursor.is_none() {
                tracing::debug!(page = page_num, "[composio:notion] no next cursor, done");
                break;
            }
        }

        // ── Step 5: advance cursor and save state ───────────────────
        if let Some(new_cursor) = newest_edited_time {
            state.advance_cursor(&new_cursor);
        }
        state.save(&memory).await?;

        let finished_at_ms = sync::now_ms();
        let summary = format!(
            "notion sync ({reason}): fetched {total_fetched}, persisted {total_persisted} new, \
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
            "[composio:notion] incremental sync complete"
        );

        Ok(SyncOutcome {
            toolkit: "notion".to_string(),
            connection_id: Some(connection_id),
            reason: reason.as_str().to_string(),
            items_ingested: total_persisted,
            started_at_ms,
            finished_at_ms,
            summary,
            details: json!({
                "results_fetched": total_fetched,
                "results_persisted": total_persisted,
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
            "[composio:notion] on_trigger"
        );
        if let Err(e) = self.sync(ctx, SyncReason::Manual).await {
            tracing::warn!(
                error = %e,
                "[composio:notion] trigger-driven sync failed (non-fatal)"
            );
        }
        Ok(())
    }
}
