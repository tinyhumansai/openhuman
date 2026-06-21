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

use super::source::run_notion_sync;
use super::sync;
use crate::openhuman::memory_sync::composio::providers::sync_state::extract_item_id;
use crate::openhuman::memory_sync::composio::providers::{
    first_array_str, merge_extra, pick_str, resolve_sync_interval_secs, ComposioProvider,
    CuratedTool, NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
    TaskContainer, TaskFetchFilter, TaskKind,
};

pub(crate) const ACTION_GET_ABOUT_ME: &str = "NOTION_GET_ABOUT_ME";
pub(crate) const ACTION_FETCH_DATA: &str = "NOTION_FETCH_DATA";
pub(crate) const ACTION_QUERY_DATABASE: &str = "NOTION_QUERY_DATABASE";
pub(crate) const ACTION_SEARCH_NOTION_PAGE: &str = "NOTION_SEARCH_NOTION_PAGE";

/// Paths for extracting a page's unique ID.
pub(crate) const PAGE_ID_PATHS: &[&str] = &["id", "data.id", "pageId", "data.pageId"];

/// Paths for extracting the `last_edited_time` used as sync cursor.
pub(crate) const PAGE_EDITED_PATHS: &[&str] = &[
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

    /// Incremental sync. Notion was the first provider migrated to the generic
    /// [`orchestrator`](crate::openhuman::memory_sync::composio::providers::orchestrator):
    /// the per-item loop, dedup, `max_items` cap, `sync_depth_days` window, and
    /// cursor handling all live in `run_sync`; the Notion-specific primitives
    /// (page fetch, dedup key, body fetch, ingest) live in [`super::source`].
    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        run_notion_sync(ctx, reason).await
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
