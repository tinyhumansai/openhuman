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

use async_trait::async_trait;
use serde_json::{json, Value};

use super::ingest::ingest_page_into_memory_tree;
use super::sync;
use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::sync_state::{extract_item_id, SyncState};
use crate::openhuman::memory_sync::composio::providers::{
    first_array_str, merge_extra, pick_str, resolve_sync_interval_secs, ComposioProvider,
    CuratedTool, NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
    TaskContainer, TaskFetchFilter, TaskKind,
};
use futures::StreamExt;

pub(crate) const ACTION_GET_ABOUT_ME: &str = "NOTION_GET_ABOUT_ME";
pub(crate) const ACTION_FETCH_DATA: &str = "NOTION_FETCH_DATA";
/// Per-page action that returns the page's rendered **body** as markdown
/// (paragraphs, headings, lists, body tables). `FETCH_DATA` only returns
/// metadata + properties; this fills in the actual document content for
/// free-form pages. One request per page (budget-counted).
pub(crate) const ACTION_GET_PAGE_MARKDOWN: &str = "NOTION_GET_PAGE_MARKDOWN";
pub(crate) const ACTION_QUERY_DATABASE: &str = "NOTION_QUERY_DATABASE";
pub(crate) const ACTION_SEARCH_NOTION_PAGE: &str = "NOTION_SEARCH_NOTION_PAGE";

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

/// Max in-flight ingests per page. DB writes serialize anyway and the
/// cloud embedder has rate limits, so keep this small.
const INGEST_CONCURRENCY: usize = 8;

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

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(super::tools::NOTION_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        Some(resolve_sync_interval_secs("notion", 30 * 60))
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
            .execute(ACTION_GET_ABOUT_ME, Some(json!({})))
            .await
            .map_err(|e| format!("[composio:notion] {ACTION_GET_ABOUT_ME} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:notion] {ACTION_GET_ABOUT_ME}: {err}"));
        }

        // `data` is already the inner Composio response payload — paths
        // here are relative to it. For bot-token connections the
        // top-level `name` is the *integration's* name (e.g. "Composio"),
        // and the actual owning user lives at `bot.owner.user.*`. Probe
        // the bot-owner paths first so identity reflects the user (#1365).
        let data = &resp.data;
        let display_name = pick_str(data, &["bot.owner.user.name", "user.name", "name"]);
        let email = pick_str(
            data,
            &[
                "bot.owner.user.person.email",
                "user.person.email",
                "person.email",
                "email",
            ],
        );
        let username = pick_str(data, &["bot.owner.user.id", "user.id", "id"]);
        let avatar_url = pick_str(
            data,
            &["bot.owner.user.avatar_url", "user.avatar_url", "avatar_url"],
        );
        let profile_url = pick_str(data, &["url", "profile_url", "profile.url"]);

        Ok(ProviderUserProfile {
            toolkit: "notion".to_string(),
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

        // ctx.max_items: route through ItemCap — page ceiling, mid-page
        // per-item break, and post-page hard stop all share one source of truth.
        let mut cap = super::super::helpers::ItemCap::new(ctx.max_items);
        let effective_max_pages = cap.max_pages(page_size, MAX_PAGES_PER_SYNC);
        if ctx.max_items.is_some() && effective_max_pages < MAX_PAGES_PER_SYNC {
            tracing::debug!(
                connection_id = %connection_id,
                max_items = ?ctx.max_items,
                effective_max_pages,
                "[composio:notion] [memory_sync] applying max_items page cap"
            );
        }

        // ctx.sync_depth_days: compute the oldest allowed edited_time string
        // for client-side skipping of stale results.
        let oldest_allowed_time: Option<String> = ctx.sync_depth_days.map(|days| {
            let floor = chrono::Utc::now() - chrono::Duration::days(days as i64);
            let s = floor.to_rfc3339();
            tracing::debug!(
                connection_id = %connection_id,
                sync_depth_days = days,
                oldest_allowed = %s,
                "[composio:notion] [memory_sync] applying sync_depth_days floor"
            );
            s
        });

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut newest_edited_time: Option<String> = None;
        let mut notion_cursor: Option<String> = None;
        // Track whether any per-item ingest failed this pass. If so, we hold
        // the persistent cursor — `last_edited_time > {cursor}` on the next
        // sync would otherwise exclude the failed item, and because the new
        // memory-tree pipeline (#2885) is delete-first, an *edited* page that
        // failed to re-ingest is left with neither old nor new chunks until
        // its next edit. Already-synced items are skipped cheaply via
        // `is_synced` on the re-fetch, so the cost of holding is minimal.
        let mut had_ingest_failures = false;
        let mut hit_cap_boundary = false;

        for page_num in 0..effective_max_pages {
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
                .execute(ACTION_FETCH_DATA, Some(args))
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

            // ── Step 4a: dedupe + decide which pages to ingest ──────
            let (mut pending, mut hit_cursor_boundary) =
                select_pending(&results, &state, &mut newest_edited_time);

            // ctx.sync_depth_days: drop items edited before the depth floor. `pending` is
            // in descending timestamp order, so truncate at the first item below the floor
            // and signal cursor-boundary so pagination stops.
            if let Some(ref floor) = oldest_allowed_time {
                if let Some(cut) = pending.iter().position(|p| {
                    p.edited_time
                        .as_deref()
                        .map(|t| t < floor.as_str())
                        .unwrap_or(false)
                }) {
                    pending.truncate(cut);
                    hit_cursor_boundary = true;
                }
            }

            // ctx.max_items: clamp the dedup'd batch to the remaining budget before ingest.
            cap.clamp_batch(&mut pending);

            // ── Step 4a.5: fetch each pending page's BODY markdown ──
            // FETCH_DATA only returned metadata + properties. Pull the
            // rendered page body (paragraphs, lists, body tables) per page so
            // free-form documents ingest as real content (multi-chunk) rather
            // than a single metadata chunk. One request per page — budget
            // counted. Runs AFTER the depth-floor + max-items cap so we only
            // fetch bodies for pages we'll actually ingest. On budget
            // exhaustion or error we leave `markdown_body` None and ingest
            // falls back to the metadata-only body.
            for (idx, p) in pending.iter_mut().enumerate() {
                if state.budget_exhausted() {
                    tracing::info!(
                        page = page_num,
                        "[composio:notion] budget exhausted before body fetch — \
                         remaining pages ingest metadata-only"
                    );
                    break;
                }
                match ctx
                    .execute(
                        ACTION_GET_PAGE_MARKDOWN,
                        Some(json!({ "page_id": p.page_id })),
                    )
                    .await
                {
                    Ok(resp) => {
                        state.record_requests(1);
                        if resp.successful {
                            p.markdown_body = sync::extract_page_markdown(&resp.data);
                            // Empirical visibility (INFO so it shows at default
                            // log level): on the first body fetch, log the
                            // markdown length + the raw envelope keys so we can
                            // confirm the response field / arg name on a live
                            // run and refine if needed.
                            if page_num == 0 && idx == 0 {
                                tracing::info!(
                                    page_id = %p.page_id,
                                    markdown_chars =
                                        p.markdown_body.as_ref().map(|s| s.len()).unwrap_or(0),
                                    raw_keys = ?resp
                                        .data
                                        .as_object()
                                        .map(|o| o.keys().cloned().collect::<Vec<_>>()),
                                    "[composio:notion] GET_PAGE_MARKDOWN sample (empirical check)"
                                );
                            }
                            if p.markdown_body.is_none() {
                                tracing::warn!(
                                    page_id = %p.page_id,
                                    "[composio:notion] GET_PAGE_MARKDOWN returned no markdown \
                                     field — metadata-only fallback"
                                );
                            }
                        } else {
                            tracing::warn!(
                                page_id = %p.page_id,
                                error = ?resp.error,
                                "[composio:notion] GET_PAGE_MARKDOWN failed — metadata-only fallback"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            page_id = %p.page_id,
                            error = %e,
                            "[composio:notion] GET_PAGE_MARKDOWN execute error — \
                             metadata-only fallback"
                        );
                    }
                }
            }

            // ── Step 4b: ingest queued pages (bounded concurrency) ──
            let ingestor = MemoryTreeIngestor {
                config: ctx.config.as_ref(),
                connection_id: &connection_id,
            };
            let outcome = ingest_pending_buffered(&ingestor, pending, INGEST_CONCURRENCY).await;
            for key in &outcome.synced_keys {
                state.mark_synced(key);
            }
            total_persisted += outcome.persisted;
            cap.record(outcome.persisted);
            if outcome.had_failures {
                had_ingest_failures = true;
            }

            // ctx.max_items precise cap: once the per-source cap is hit, stop paginating.
            if cap.is_reached() {
                hit_cap_boundary = true;
                break;
            }

            if hit_cursor_boundary {
                tracing::debug!(
                    page = page_num,
                    "[composio:notion] reached cursor boundary, stopping"
                );
                break;
            }

            // ctx.max_items hard stop.
            if cap.is_reached() {
                tracing::debug!(
                    page = page_num,
                    total_persisted,
                    "[composio:notion] [memory_sync] max_items reached, stopping pagination"
                );
                hit_cap_boundary = true;
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
        //
        // Hold the cursor when any item failed to ingest this pass. See the
        // `had_ingest_failures` declaration above for why this matters under
        // the delete-first memory-tree pipeline (#2885).
        // Hold the cursor on a cap-truncated pass so the next sync re-scans the unseen tail.
        if !had_ingest_failures && !hit_cap_boundary {
            if let Some(new_cursor) = newest_edited_time {
                state.advance_cursor(&new_cursor);
            }
        } else {
            tracing::warn!(
                connection_id = %connection_id,
                had_ingest_failures,
                hit_cap_boundary,
                "[composio:notion] holding cursor — ingest failures or cap-truncated pass; next \
                 sync will re-fetch the failed range"
            );
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

    async fn fetch_tasks(
        &self,
        ctx: &ProviderContext,
        filter: &TaskFetchFilter,
    ) -> Result<Vec<NormalizedTask>, String> {
        let max = filter.effective_max();
        let database_id = filter
            .database_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        tracing::debug!(
            connection_id = ?ctx.connection_id,
            max,
            has_database = database_id.is_some(),
            "[composio:notion] fetch_tasks"
        );

        // A configured board (database) uses NOTION_QUERY_DATABASE;
        // otherwise fall back to NOTION_FETCH_DATA (recent pages), the
        // same action the periodic sync uses.
        let (action, mut args) = match database_id {
            Some(db) => (
                ACTION_QUERY_DATABASE,
                json!({
                    "database_id": db,
                    "page_size": max.min(100) as u32,
                    "sorts": [ { "timestamp": "last_edited_time", "direction": "descending" } ],
                }),
            ),
            None => (
                ACTION_FETCH_DATA,
                json!({
                    "page_size": max.min(100) as u32,
                    "filter": { "value": "page", "property": "object" },
                    "sort": { "direction": "descending", "timestamp": "last_edited_time" },
                }),
            ),
        };
        merge_extra(&mut args, &filter.extra);

        let resp = ctx
            .execute(action, Some(args))
            .await
            .map_err(|e| format!("[composio:notion] {action}: {e:#}"))?;
        if !resp.successful {
            return Err(format!(
                "[composio:notion] {action}: {}",
                resp.error.unwrap_or_else(|| "provider failure".into())
            ));
        }

        // Optional client-side status filter — Notion status properties
        // are user-defined, so we match on the normalized status rather
        // than building a server-side property filter.
        let want_status = filter
            .status
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());

        let mut out: Vec<NormalizedTask> = Vec::new();
        for page in sync::extract_results(&resp.data) {
            if out.len() >= max {
                break;
            }
            let Some(nt) = normalize_notion_page(&page) else {
                continue;
            };
            if let Some(ref want) = want_status {
                let matches = nt
                    .status
                    .as_deref()
                    .map(|s| s.to_ascii_lowercase() == *want)
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            out.push(nt);
        }
        tracing::debug!(count = out.len(), "[composio:notion] fetch_tasks complete");
        Ok(out)
    }

    /// List the Notion databases (tables) the connected integration can see,
    /// via `NOTION_SEARCH_NOTION_PAGE` filtered to database objects, so the
    /// task-source UI can offer a picker for `database_id`. Only databases the
    /// integration has been *shared with* in Notion are returned.
    async fn list_databases(&self, ctx: &ProviderContext) -> Result<Vec<TaskContainer>, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:notion] list_databases via {ACTION_SEARCH_NOTION_PAGE}"
        );
        // Composio's NOTION_SEARCH_NOTION_PAGE *flattens* Notion's native
        // `filter: { value, property }` into top-level `filter_value` /
        // `filter_property` params and silently drops the nested form (which
        // returned only pages). We send the flat params here; the nested
        // `filter` is kept too as a belt-and-braces hint for any variant that
        // honours it, and the parser still drops any stray `page` items.
        let args = json!({
            "query": "",
            "filter_value": "database",
            "filter_property": "object",
            "filter": { "value": "database", "property": "object" },
            "page_size": 100,
        });
        let resp = ctx
            .execute(ACTION_SEARCH_NOTION_PAGE, Some(args))
            .await
            .map_err(|e| format!("[composio:notion] {ACTION_SEARCH_NOTION_PAGE}: {e:#}"))?;
        if !resp.successful {
            return Err(format!(
                "[composio:notion] {ACTION_SEARCH_NOTION_PAGE}: {}",
                resp.error.unwrap_or_else(|| "provider failure".into())
            ));
        }

        tracing::info!(
            successful = resp.successful,
            data_is_array = resp.data.is_array(),
            data_keys = ?resp.data.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
            "[composio:notion] list_databases raw response shape"
        );
        let out = parse_database_results(&resp.data);
        tracing::info!(
            count = out.len(),
            "[composio:notion] list_databases complete"
        );
        Ok(out)
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

/// Map a raw Notion page payload into a [`NormalizedTask`].
///
/// Notion databases are user-defined, so property extraction is
/// best-effort against common property names (`Status`, `Assignee`,
/// `Due`). Anything unmatched is simply left `None` — the raw payload is
/// preserved for enrichment.
fn normalize_notion_page(page: &serde_json::Value) -> Option<NormalizedTask> {
    let external_id = extract_item_id(page, PAGE_ID_PATHS)?;
    let title =
        sync::extract_page_title(page).unwrap_or_else(|| format!("Notion page {external_id}"));
    Some(NormalizedTask {
        external_id,
        source_id: String::new(),
        provider: "notion".to_string(),
        kind: TaskKind::Generic,
        title,
        body: None,
        url: pick_str(page, &["url", "data.url"]),
        status: pick_str(
            page,
            &[
                "properties.Status.status.name",
                "properties.Status.select.name",
                "data.properties.Status.status.name",
            ],
        ),
        assignee: first_array_str(
            page,
            &[
                "properties.Assignee.people",
                "data.properties.Assignee.people",
            ],
            &["name"],
        ),
        due: pick_str(
            page,
            &[
                "properties.Due.date.start",
                "data.properties.Due.date.start",
            ],
        ),
        labels: Vec::new(),
        priority: pick_str(
            page,
            &[
                "properties.Priority.select.name",
                "data.properties.Priority.select.name",
            ],
        ),
        updated_at: extract_item_id(page, PAGE_EDITED_PATHS),
        raw: page.clone(),
    })
}

/// Map a `NOTION_SEARCH_NOTION_PAGE` response into the database containers
/// the UI picker needs.
///
/// We send a server-side `object: database` filter, so the response is
/// already scoped — we therefore *trust* it and only drop items explicitly
/// typed as `page`. This is intentional: Composio's response items don't
/// always carry a top-level `object` field, and an over-strict
/// "keep only object==database" check silently dropped every database.
/// Pure (no I/O) so it is unit-testable.
pub(super) fn parse_database_results(data: &serde_json::Value) -> Vec<TaskContainer> {
    let results = sync::extract_results(data);
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut out: Vec<TaskContainer> = Vec::new();
    for item in &results {
        let object = pick_str(item, &["object", "data.object"]);
        *kinds
            .entry(object.clone().unwrap_or_else(|| "<none>".to_string()))
            .or_default() += 1;
        // Trust the server-side database filter: keep databases / data_sources
        // *and* objectless items; only drop items explicitly typed as pages.
        if object.as_deref() == Some("page") {
            continue;
        }
        let Some(id) = extract_item_id(item, PAGE_ID_PATHS) else {
            continue;
        };
        let title = extract_database_title(item).unwrap_or_else(|| format!("Notion database {id}"));
        out.push(TaskContainer { id, title });
    }
    tracing::info!(
        raw = results.len(),
        kept = out.len(),
        object_kinds = ?kinds,
        "[composio:notion] parse_database_results"
    );
    out
}

/// Extract a Notion database's display title from its top-level `title`
/// rich-text array (`title[].plain_text`), tolerant of the Composio `data`
/// wrapper. Returns `None` for an untitled / shapeless database.
fn extract_database_title(db: &serde_json::Value) -> Option<String> {
    let arr = db
        .get("title")
        .or_else(|| db.get("data").and_then(|d| d.get("title")))
        .and_then(|v| v.as_array())?;
    let text: String = arr
        .iter()
        .filter_map(|t| {
            t.get("plain_text").and_then(|p| p.as_str()).or_else(|| {
                t.get("text")
                    .and_then(|x| x.get("content"))
                    .and_then(|c| c.as_str())
            })
        })
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// One page that passed dedupe in pass 1 and is queued for concurrent
/// ingest in pass 2. Borrows the raw page `Value` out of the current
/// page's `results` (same scope — no clone needed).
struct PendingIngest<'a> {
    sync_key: String,
    page_id: String,
    title: String,
    edited_time: Option<String>,
    page: &'a Value,
    /// Rendered page body (markdown) fetched per-page via
    /// `NOTION_GET_PAGE_MARKDOWN` after dedupe, before ingest. `None` when the
    /// body fetch was skipped (budget) or failed — ingest falls back to the
    /// metadata-only body.
    markdown_body: Option<String>,
}

/// Folded result of [`ingest_pending_buffered`]. Every field is
/// order-independent, so the concurrent stage can accumulate into it
/// regardless of the order ingests complete.
#[derive(Default)]
struct BufferedIngestOutcome {
    /// `sync_key`s whose ingest succeeded — the caller marks each synced.
    synced_keys: Vec<String>,
    /// Number of pages persisted (equals `synced_keys.len()`).
    persisted: usize,
    /// Whether any per-item ingest failed (the caller holds the cursor).
    had_failures: bool,
}

/// Seam over "ingest one Notion page", so the bounded-concurrency driver
/// can be unit-tested with a fake that records peak in-flight calls
/// without a real memory tree or embedder.
#[async_trait]
trait PageIngestor {
    async fn ingest(
        &self,
        page_id: &str,
        title: &str,
        edited_time: Option<&str>,
        page: &Value,
        markdown_body: Option<&str>,
    ) -> anyhow::Result<usize>;
}

/// Production ingestor: routes into the memory-tree pipeline (#2885) via
/// [`ingest_page_into_memory_tree`].
struct MemoryTreeIngestor<'c> {
    config: &'c Config,
    connection_id: &'c str,
}

#[async_trait]
impl PageIngestor for MemoryTreeIngestor<'_> {
    async fn ingest(
        &self,
        page_id: &str,
        title: &str,
        edited_time: Option<&str>,
        page: &Value,
        markdown_body: Option<&str>,
    ) -> anyhow::Result<usize> {
        ingest_page_into_memory_tree(
            self.config,
            self.connection_id,
            page_id,
            title,
            edited_time,
            page,
            markdown_body,
        )
        .await
    }
}

/// Pass 1 (pure, no I/O): scan one page of `results`, advance
/// `newest_edited_time`, skip already-synced items, and collect the
/// pages still needing ingest. Returns the queued items and whether we
/// crossed the persistent cursor boundary (the signal to stop
/// paginating). All order-dependent decisions (cursor/timestamp) live
/// here — never in the concurrent stage.
fn select_pending<'a>(
    results: &'a [Value],
    state: &SyncState,
    newest_edited_time: &mut Option<String>,
) -> (Vec<PendingIngest<'a>>, bool) {
    let mut hit_cursor_boundary = false;
    let mut pending: Vec<PendingIngest> = Vec::new();
    for page in results {
        let Some(page_id) = extract_item_id(page, PAGE_ID_PATHS) else {
            tracing::debug!("[composio:notion] page missing ID, skipping");
            continue;
        };

        let edited_time = extract_item_id(page, PAGE_EDITED_PATHS);

        // Track the newest edited time for cursor advancement.
        if let Some(ref et) = edited_time {
            if newest_edited_time
                .as_ref()
                .is_none_or(|existing| et > existing)
            {
                *newest_edited_time = Some(et.clone());
            }
        }

        let sync_key = match &edited_time {
            Some(et) => format!("{page_id}@{et}"),
            None => page_id.clone(),
        };

        // Older than cursor AND already synced → caught up.
        if let (Some(ref cursor), Some(ref et)) = (&state.cursor, &edited_time) {
            if et <= cursor && state.is_synced(&sync_key) {
                hit_cursor_boundary = true;
                continue;
            }
        }

        if state.is_synced(&sync_key) {
            continue;
        }

        let title_text =
            sync::extract_page_title(page).unwrap_or_else(|| format!("Notion page {page_id}"));
        let title = format!("Notion: {title_text}");

        pending.push(PendingIngest {
            sync_key,
            page_id,
            title,
            edited_time,
            page,
            markdown_body: None,
        });
    }
    (pending, hit_cursor_boundary)
}

/// Pass 2: ingest the queued pages with bounded concurrency. Overlaps
/// the per-item embedding RTT (`buffer_unordered`, up to `concurrency`
/// in flight) and folds results into an order-independent
/// [`BufferedIngestOutcome`]. Unordered is correct here (unlike the
/// order-aligned GitHub reader): nothing downstream depends on
/// completion order — successes are keyed by `sync_key`.
async fn ingest_pending_buffered<I: PageIngestor + Sync>(
    ingestor: &I,
    pending: Vec<PendingIngest<'_>>,
    concurrency: usize,
) -> BufferedIngestOutcome {
    // Materialize the per-item futures into a Vec before `buffer_unordered`
    // so the spawned sync future keeps concrete lifetimes / `Send`.
    let ingest_futs = pending
        .into_iter()
        .map(|p| async move {
            let res = ingestor
                .ingest(
                    &p.page_id,
                    &p.title,
                    p.edited_time.as_deref(),
                    p.page,
                    p.markdown_body.as_deref(),
                )
                .await;
            (p.sync_key, p.page_id, res)
        })
        .collect::<Vec<_>>();

    let mut outcome = BufferedIngestOutcome::default();
    let mut ingest_stream = futures::stream::iter(ingest_futs).buffer_unordered(concurrency);
    while let Some((sync_key, page_id, res)) = ingest_stream.next().await {
        match res {
            Ok(_chunks_written) => {
                outcome.synced_keys.push(sync_key);
                outcome.persisted += 1;
            }
            Err(e) => {
                outcome.had_failures = true;
                tracing::warn!(
                    page_id = %page_id,
                    error = %e,
                    "[composio:notion] failed to ingest page into memory_tree (continuing)"
                );
            }
        }
    }
    outcome
}

#[cfg(test)]
mod buffered_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Fake ingestor: records the peak number of concurrent in-flight
    /// `ingest` calls and can be told to fail one specific `page_id`. No
    /// memory tree or embedder involved — lets us assert the concurrency
    /// bound and overlap deterministically.
    struct CountingIngestor {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        fail_page: Option<String>,
    }

    impl CountingIngestor {
        fn new(fail_page: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                fail_page: fail_page.map(str::to_string),
            })
        }
    }

    #[async_trait]
    impl PageIngestor for CountingIngestor {
        async fn ingest(
            &self,
            page_id: &str,
            _title: &str,
            _edited_time: Option<&str>,
            _page: &Value,
            _markdown_body: Option<&str>,
        ) -> anyhow::Result<usize> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            // Yield a few times so futures genuinely interleave and the
            // peak counter reflects real overlap, not accidental serial run.
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if self.fail_page.as_deref() == Some(page_id) {
                Err(anyhow::anyhow!("forced failure for {page_id}"))
            } else {
                Ok(1)
            }
        }
    }

    fn make_pages(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({ "id": format!("p{i}") })).collect()
    }

    fn make_pending<'a>(pages: &'a [Value]) -> Vec<PendingIngest<'a>> {
        pages
            .iter()
            .enumerate()
            .map(|(i, page)| PendingIngest {
                sync_key: format!("k{i}"),
                page_id: format!("p{i}"),
                title: format!("Notion: page {i}"),
                edited_time: None,
                page,
                markdown_body: None,
            })
            .collect()
    }

    #[tokio::test]
    async fn ingest_pending_buffered_bounds_and_overlaps() {
        let ingestor = CountingIngestor::new(None);
        let pages = make_pages(20);
        let pending = make_pending(&pages);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, 8).await;

        assert_eq!(outcome.persisted, 20, "all pages persisted");
        assert_eq!(outcome.synced_keys.len(), 20);
        assert!(!outcome.had_failures);

        let peak = ingestor.peak.load(Ordering::SeqCst);
        assert!(peak <= 8, "peak in-flight {peak} exceeded the bound of 8");
        assert!(peak >= 2, "peak in-flight {peak} shows no real overlap");
    }

    #[tokio::test]
    async fn ingest_pending_buffered_skips_failures_order_independent() {
        let ingestor = CountingIngestor::new(Some("p2"));
        let pages = make_pages(5);
        let pending = make_pending(&pages);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, 4).await;

        assert_eq!(outcome.persisted, 4, "the one failed ingest is not counted");
        assert!(outcome.had_failures);
        assert_eq!(outcome.synced_keys.len(), 4);
        assert!(
            !outcome.synced_keys.contains(&"k2".to_string()),
            "the failed page's sync_key must not be marked synced"
        );
    }

    #[test]
    fn select_pending_tracks_newest_skips_synced_and_detects_boundary() {
        let mut state = SyncState::new("notion", "conn1");
        state.cursor = Some("2026-04-15T00:00:00Z".to_string());
        // Page B is already synced and older than the cursor.
        state.mark_synced("b@2026-04-01T00:00:00Z");

        let pages = vec![
            json!({ "id": "a", "last_edited_time": "2026-05-01T00:00:00Z" }),
            json!({ "id": "b", "last_edited_time": "2026-04-01T00:00:00Z" }),
            json!({ "last_edited_time": "2026-03-01T00:00:00Z" }), // no id → skipped
        ];

        let mut newest: Option<String> = None;
        let (pending, hit_boundary) = select_pending(&pages, &state, &mut newest);

        assert_eq!(pending.len(), 1, "only the new page A is queued");
        assert_eq!(pending[0].page_id, "a");
        assert_eq!(pending[0].sync_key, "a@2026-05-01T00:00:00Z");
        assert!(
            hit_boundary,
            "older synced page B trips the cursor boundary"
        );
        assert_eq!(newest.as_deref(), Some("2026-05-01T00:00:00Z"));
    }
}
