//! ClickUp provider — incremental sync of tasks assigned to the
//! authenticated user, with per-item persistence into the Memory Tree.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. If we don't yet know the user's numeric ID, call
//!      `CLICKUP_GET_AUTHORIZED_USER` and cache the result in memory
//!      (it doesn't change for the lifetime of the connection).
//!   4. If we don't yet know which workspaces (teams) the connection
//!      can see, call `CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES` and
//!      cache the list.
//!   5. For each workspace, page through
//!      `CLICKUP_GET_FILTERED_TEAM_TASKS` filtered to the user as
//!      assignee, sorted by `date_updated` descending. Stop a workspace
//!      early once we hit tasks older than the cursor.
//!   6. For each task, persist as a single memory document if it's new
//!      *or* edited since the last sync.
//!   7. Advance the cursor to the newest `date_updated` seen and save.
//!
//! Privacy posture: we only pull tasks the user is assigned to, never
//! the whole workspace's task graph. This mirrors the
//! "fetch-what-the-user-sees" model `gmail` / `notion` already follow
//! and avoids accidentally ingesting other teammates' private tasks.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use super::{ingest::ingest_task_into_memory_tree, sync};
use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::sync_state::{extract_item_id, SyncState};
use crate::openhuman::memory_sync::composio::providers::{
    first_array_str, merge_extra, pick_str, resolve_sync_interval_secs, ComposioProvider,
    CuratedTool, NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
    TaskFetchFilter, TaskKind,
};

pub(crate) const ACTION_GET_AUTHORIZED_USER: &str = "CLICKUP_GET_AUTHORIZED_USER";
pub(crate) const ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES: &str =
    "CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES";
pub(crate) const ACTION_GET_FILTERED_TEAM_TASKS: &str = "CLICKUP_GET_FILTERED_TEAM_TASKS";

/// Page size per API call. ClickUp's filtered-team-tasks endpoint
/// returns up to 100 tasks per page; we ask for a smaller window on
/// steady-state syncs to keep response sizes bounded.
const PAGE_SIZE: u32 = 50;

/// Larger initial-sync page size, used immediately after OAuth so the
/// first backfill catches up faster.
const INITIAL_PAGE_SIZE: u32 = 100;

/// Maximum pages (per workspace) per sync pass before yielding. Caps
/// initial backfill churn — anything beyond this rolls over to the
/// next sync interval.
const MAX_PAGES_PER_WORKSPACE: u32 = 20;

/// Paths for extracting a task's unique ID. Composio sometimes wraps
/// the upstream payload under `data`, so we check both shapes.
const TASK_ID_PATHS: &[&str] = &["id", "data.id", "task_id", "data.task_id"];

/// Max in-flight ingests per page. DB writes serialize anyway and the
/// cloud embedder has rate limits, so keep this small.
const INGEST_CONCURRENCY: usize = 8;

pub struct ClickUpProvider;

impl ClickUpProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClickUpProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for ClickUpProvider {
    fn toolkit_slug(&self) -> &'static str {
        "clickup"
    }

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(super::tools::CLICKUP_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        // 30 minutes — same cadence as Notion. ClickUp tasks change
        // more slowly than chat but faster than email, so this is in
        // the middle.
        Some(resolve_sync_interval_secs("clickup", 30 * 60))
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:clickup] fetch_user_profile via {ACTION_GET_AUTHORIZED_USER}"
        );

        let resp = ctx
            .execute(ACTION_GET_AUTHORIZED_USER, Some(json!({})))
            .await
            .map_err(|e| {
                format!("[composio:clickup] {ACTION_GET_AUTHORIZED_USER} failed: {e:#}")
            })?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:clickup] {ACTION_GET_AUTHORIZED_USER}: {err}"
            ));
        }

        // Composio's wrapping puts ClickUp's `{user: {…}}` payload at
        // `data` or `data.user`. We probe both — `pick_str` walks dotted
        // paths so `user.username` and `data.user.username` both work.
        let data = &resp.data;
        let display_name = pick_str(data, &["user.username", "data.user.username", "username"]);
        let email = pick_str(data, &["user.email", "data.user.email", "email"]);
        let username = sync::extract_user_id(data);
        let avatar_url = pick_str(
            data,
            &[
                "user.profilePicture",
                "data.user.profilePicture",
                "profilePicture",
            ],
        );
        let profile_url = None;

        Ok(ProviderUserProfile {
            toolkit: "clickup".to_string(),
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
            "[composio:clickup] incremental sync starting"
        );

        // ── Step 1: load persistent sync state ──────────────────────
        let Some(memory) = ctx.memory_client() else {
            return Err("[composio:clickup] memory client not ready".to_string());
        };
        let mut state = SyncState::load(&memory, "clickup", &connection_id).await?;

        // ── Step 2: check daily budget ──────────────────────────────
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:clickup] daily request budget exhausted, skipping sync"
            );
            return Ok(SyncOutcome {
                toolkit: "clickup".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "clickup sync skipped: daily budget exhausted".to_string(),
                details: json!({ "budget_exhausted": true }),
            });
        }

        // ── Step 3: resolve the authenticated user's numeric ID ─────
        //
        // ClickUp's "filtered team tasks" endpoint accepts an
        // `assignees` filter as a list of user IDs. We need the
        // *current* user's ID to scope the sync to "my tasks" rather
        // than "everyone's tasks". The ID is stable for the lifetime
        // of the OAuth connection, so we only fetch it once per sync
        // pass (and cheaply re-fetch each pass — Composio caches and
        // the call is sub-100ms).
        let user_id = match self.resolve_user_id(ctx, &mut state).await {
            Ok(id) => id,
            Err(e) => {
                let _ = state.save(&memory).await;
                return Err(e);
            }
        };

        // Re-check the budget here — `resolve_user_id` just spent one
        // request, and if that pushed us over the cap, firing
        // `CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES` would be wasted
        // work. Bailing here keeps the per-day API call count strictly
        // honoured even when we entered the sync with one slot left.
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:clickup] budget exhausted after user-id probe, skipping sync"
            );
            state.save(&memory).await?;
            return Ok(SyncOutcome {
                toolkit: "clickup".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "clickup sync skipped: daily budget exhausted after user-id probe"
                    .to_string(),
                details: json!({ "budget_exhausted": true, "user_id_resolved": true }),
            });
        }

        // ── Step 4: resolve which workspaces (teams) to iterate ─────
        let workspaces = match self.resolve_workspaces(ctx, &mut state).await {
            Ok(ws) => ws,
            Err(e) => {
                let _ = state.save(&memory).await;
                return Err(e);
            }
        };

        if workspaces.is_empty() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:clickup] no workspaces visible to this connection; nothing to sync"
            );
            state.save(&memory).await?;
            return Ok(SyncOutcome {
                toolkit: "clickup".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "clickup sync: no workspaces visible".to_string(),
                details: json!({ "workspaces_visible": 0 }),
            });
        }

        // ── Step 5: paginated incremental fetch per workspace ───────
        let page_size = match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        };

        // ctx.max_items: route through ItemCap — page ceiling, mid-page
        // per-item break, and post-page hard stop all share one source of truth.
        let mut cap = super::super::helpers::ItemCap::new(ctx.max_items);
        let effective_max_pages = cap.max_pages(page_size, MAX_PAGES_PER_WORKSPACE);
        if ctx.max_items.is_some() && effective_max_pages < MAX_PAGES_PER_WORKSPACE {
            tracing::debug!(
                connection_id = %connection_id,
                max_items = ?ctx.max_items,
                effective_max_pages,
                "[composio:clickup] [memory_sync] applying max_items page cap"
            );
        }

        // ctx.sync_depth_days: oldest allowed date_updated for client-side skip.
        // ClickUp's `date_updated` field is a millisecond-epoch string, so the
        // floor must also be epoch-millis (not RFC3339) for the lexicographic
        // compare in `select_pending` to work correctly.
        let oldest_allowed_time: Option<String> = ctx.sync_depth_days.map(|days| {
            let floor = chrono::Utc::now() - chrono::Duration::days(days as i64);
            let s = floor.timestamp_millis().to_string();
            tracing::debug!(
                connection_id = %connection_id,
                sync_depth_days = days,
                oldest_allowed_ms = %s,
                "[composio:clickup] [memory_sync] applying sync_depth_days floor"
            );
            s
        });

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut newest_updated: Option<String> = None;
        let mut hit_cap_boundary = false;

        'workspaces: for workspace_id in &workspaces {
            for page_num in 0..effective_max_pages {
                if state.budget_exhausted() {
                    tracing::info!(
                        workspace_id = %workspace_id,
                        page = page_num,
                        "[composio:clickup] budget exhausted mid-sync, stopping pagination"
                    );
                    break 'workspaces;
                }

                let args = json!({
                    "team_id": workspace_id,
                    "assignees": [user_id.clone()],
                    "order_by": "updated",
                    "reverse": true,
                    "page": page_num,
                    "page_size": page_size,
                    // Include subtasks so per-list "checklist" style work
                    // also reaches Memory Tree.
                    "subtasks": true,
                    // Include archived = false (default) — we don't want
                    // closed-and-archived noise in memory.
                });

                let resp = ctx
                    .execute(ACTION_GET_FILTERED_TEAM_TASKS, Some(args))
                    .await
                    .map_err(|e| {
                        format!(
                            "[composio:clickup] {ACTION_GET_FILTERED_TEAM_TASKS} \
                             workspace={workspace_id} page={page_num}: {e:#}"
                        )
                    })?;

                state.record_requests(1);

                if !resp.successful {
                    let err = resp
                        .error
                        .clone()
                        .unwrap_or_else(|| "provider reported failure".to_string());
                    let _ = state.save(&memory).await;
                    return Err(format!(
                        "[composio:clickup] {ACTION_GET_FILTERED_TEAM_TASKS} \
                         workspace={workspace_id} page={page_num}: {err}"
                    ));
                }

                let tasks = sync::extract_tasks(&resp.data);
                total_fetched += tasks.len();

                if tasks.is_empty() {
                    tracing::debug!(
                        workspace_id = %workspace_id,
                        page = page_num,
                        "[composio:clickup] empty page, moving to next workspace"
                    );
                    break;
                }

                // ── Per-item dedup + bounded-concurrency ingest ─────
                let (mut pending, mut hit_cursor_boundary) =
                    select_pending(&tasks, &state, &mut newest_updated);

                // ctx.sync_depth_days: drop items updated before the depth floor. `pending` is
                // in descending timestamp order, so truncate at the first item below the floor
                // and signal cursor-boundary so pagination stops.
                if let Some(ref floor) = oldest_allowed_time {
                    if let Some(cut) = pending.iter().position(|p| {
                        p.updated
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

                let ingestor = MemoryTreeIngestor {
                    config: ctx.config.as_ref(),
                    connection_id: &connection_id,
                };
                let outcome =
                    ingest_pending_buffered(&ingestor, pending, workspace_id, INGEST_CONCURRENCY)
                        .await;
                for key in &outcome.synced_keys {
                    state.mark_synced(key);
                }
                total_persisted += outcome.persisted;
                cap.record(outcome.persisted);

                // ctx.max_items precise cap: once the per-source cap is hit, stop paginating.
                if cap.is_reached() {
                    hit_cap_boundary = true;
                    break 'workspaces;
                }

                if hit_cursor_boundary {
                    tracing::debug!(
                        workspace_id = %workspace_id,
                        page = page_num,
                        "[composio:clickup] reached cursor boundary, stopping workspace"
                    );
                    break;
                }

                // ctx.max_items hard stop.
                if cap.is_reached() {
                    tracing::debug!(
                        workspace_id = %workspace_id,
                        page = page_num,
                        total_persisted,
                        "[composio:clickup] [memory_sync] max_items reached, stopping pagination"
                    );
                    hit_cap_boundary = true;
                    break 'workspaces;
                }

                // ClickUp's filtered-team-tasks endpoint signals the last
                // page implicitly: when fewer than `page_size` results
                // come back, there are no more pages.
                if (tasks.len() as u32) < page_size {
                    tracing::debug!(
                        workspace_id = %workspace_id,
                        page = page_num,
                        returned = tasks.len(),
                        "[composio:clickup] short page, end of workspace"
                    );
                    break;
                }
            }
        }

        // ── Step 6: advance cursor and save state ───────────────────
        // Hold the cursor on a cap-truncated pass so the next sync re-scans the unseen tail.
        if !hit_cap_boundary {
            if let Some(new_cursor) = newest_updated {
                state.advance_cursor(&new_cursor);
            }
        } else {
            tracing::warn!(
                connection_id = %connection_id,
                "[composio:clickup] holding cursor — cap-truncated pass; next sync will re-scan \
                 the unseen tail"
            );
        }
        state.set_last_sync_at_ms(sync::now_ms());
        state.save(&memory).await?;

        let finished_at_ms = sync::now_ms();
        let summary = format!(
            "clickup sync ({reason}): fetched {total_fetched}, persisted {total_persisted} new, \
             across {workspace_count} workspace(s), budget remaining {remaining}",
            reason = reason.as_str(),
            workspace_count = workspaces.len(),
            remaining = state.budget_remaining(),
        );
        tracing::info!(
            connection_id = %connection_id,
            elapsed_ms = finished_at_ms.saturating_sub(started_at_ms),
            total_fetched,
            total_persisted,
            workspace_count = workspaces.len(),
            budget_remaining = state.budget_remaining(),
            "[composio:clickup] incremental sync complete"
        );

        Ok(SyncOutcome {
            toolkit: "clickup".to_string(),
            connection_id: Some(connection_id),
            reason: reason.as_str().to_string(),
            items_ingested: total_persisted,
            started_at_ms,
            finished_at_ms,
            summary,
            details: json!({
                "tasks_fetched": total_fetched,
                "tasks_persisted": total_persisted,
                "workspaces_visible": workspaces.len(),
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
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            max,
            team_id = ?filter.team_id,
            assignee_is_me = filter.assignee_is_me,
            "[composio:clickup] fetch_tasks"
        );

        // Resolve which workspaces (teams) to query. An explicit
        // `team_id` from the filter wins; otherwise enumerate every
        // workspace the connection can see.
        let workspaces = match &filter.team_id {
            Some(team) if !team.trim().is_empty() => vec![team.trim().to_string()],
            _ => {
                let resp = ctx
                    .execute(ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES, Some(json!({})))
                    .await
                    .map_err(|e| {
                        format!(
                            "[composio:clickup] {ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES}: {e:#}"
                        )
                    })?;
                if !resp.successful {
                    return Err(format!(
                        "[composio:clickup] {ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES}: {}",
                        resp.error.unwrap_or_else(|| "provider failure".into())
                    ));
                }
                sync::extract_workspace_ids(&resp.data)
            }
        };

        // Resolve the current user id only when the filter scopes to
        // "assigned to me".
        let assignees: Vec<String> = if filter.assignee_is_me {
            let resp = ctx
                .execute(ACTION_GET_AUTHORIZED_USER, Some(json!({})))
                .await
                .map_err(|e| format!("[composio:clickup] {ACTION_GET_AUTHORIZED_USER}: {e:#}"))?;
            // Fail closed: if we can't resolve the user, error rather than
            // silently dropping the assignee filter and fetching the whole
            // workspace's tasks.
            if !resp.successful {
                return Err(format!(
                    "[composio:clickup] {ACTION_GET_AUTHORIZED_USER}: {}",
                    resp.error.unwrap_or_else(|| "provider failure".into())
                ));
            }
            let id = sync::extract_user_id(&resp.data).ok_or_else(|| {
                "[composio:clickup] CLICKUP_GET_AUTHORIZED_USER returned no user.id".to_string()
            })?;
            vec![id]
        } else {
            Vec::new()
        };

        let mut out: Vec<NormalizedTask> = Vec::new();
        'workspaces: for workspace_id in &workspaces {
            let mut args = json!({
                "team_id": workspace_id,
                "order_by": "updated",
                "reverse": true,
                "page": 0,
                "page_size": max.min(100) as u32,
                "subtasks": true,
            });
            if !assignees.is_empty() {
                args["assignees"] = json!(assignees);
            }
            if let Some(list_id) = filter.list_id.as_deref().filter(|s| !s.trim().is_empty()) {
                args["list_ids"] = json!([list_id]);
            }
            merge_extra(&mut args, &filter.extra);

            let resp = ctx
                .execute(ACTION_GET_FILTERED_TEAM_TASKS, Some(args))
                .await
                .map_err(|e| {
                    format!("[composio:clickup] {ACTION_GET_FILTERED_TEAM_TASKS} ws={workspace_id}: {e:#}")
                })?;
            if !resp.successful {
                return Err(format!(
                    "[composio:clickup] {ACTION_GET_FILTERED_TEAM_TASKS} ws={workspace_id}: {}",
                    resp.error.unwrap_or_else(|| "provider failure".into())
                ));
            }

            for task in sync::extract_tasks(&resp.data) {
                if out.len() >= max {
                    break 'workspaces;
                }
                if let Some(nt) = normalize_clickup_task(&task) {
                    out.push(nt);
                }
            }
        }

        tracing::debug!(count = out.len(), "[composio:clickup] fetch_tasks complete");
        Ok(out)
    }
}

/// Map a raw ClickUp task payload into a [`NormalizedTask`]. Returns
/// `None` only when the task has no extractable id (unroutable).
fn normalize_clickup_task(task: &serde_json::Value) -> Option<NormalizedTask> {
    let external_id =
        crate::openhuman::memory_sync::composio::providers::sync_state::extract_item_id(
            task,
            TASK_ID_PATHS,
        )?;
    let title =
        sync::extract_task_name(task).unwrap_or_else(|| format!("ClickUp task {external_id}"));
    Some(NormalizedTask {
        external_id,
        source_id: String::new(),
        provider: "clickup".to_string(),
        kind: TaskKind::Generic,
        title,
        body: pick_str(task, &["description", "data.description", "text_content"]),
        url: pick_str(task, &["url", "data.url"]),
        status: pick_str(task, &["status.status", "data.status.status", "status"]),
        assignee: first_array_str(
            task,
            &["assignees", "data.assignees"],
            &["username", "email"],
        ),
        due: pick_str(task, &["due_date", "data.due_date"]),
        labels: Vec::new(),
        priority: pick_str(task, &["priority.priority", "data.priority.priority"]),
        updated_at: sync::extract_task_updated(task),
        raw: task.clone(),
    })
}

impl ClickUpProvider {
    /// Look up (and budget-record) the authorized user's numeric ID.
    ///
    /// The ID is stable for the connection's lifetime, but we re-fetch
    /// on every sync rather than persisting it because (a) the ClickUp
    /// API call is cheap, (b) caching it in `SyncState` would inflate
    /// the public struct for a single provider's quirk, and (c) it
    /// implicitly validates that the OAuth connection is still good
    /// before we start paginating.
    async fn resolve_user_id(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<String, String> {
        let resp = ctx
            .execute(ACTION_GET_AUTHORIZED_USER, Some(json!({})))
            .await
            .map_err(|e| {
                format!("[composio:clickup] {ACTION_GET_AUTHORIZED_USER} failed: {e:#}")
            })?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:clickup] {ACTION_GET_AUTHORIZED_USER}: {err}"
            ));
        }

        sync::extract_user_id(&resp.data).ok_or_else(|| {
            "[composio:clickup] CLICKUP_GET_AUTHORIZED_USER returned no user.id".to_string()
        })
    }

    /// Look up the list of workspace (team) IDs visible to this
    /// connection. ClickUp's per-team task-filter endpoint requires a
    /// concrete `team_id`, so we have to enumerate.
    async fn resolve_workspaces(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<Vec<String>, String> {
        let resp = ctx
            .execute(ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES, Some(json!({})))
            .await
            .map_err(|e| {
                format!("[composio:clickup] {ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES} failed: {e:#}")
            })?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:clickup] {ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES}: {err}"
            ));
        }

        Ok(sync::extract_workspace_ids(&resp.data))
    }
}

/// One task that passed dedupe in pass 1 and is queued for concurrent
/// ingest in pass 2. Borrows the raw task `Value` out of the current
/// page's `tasks` (same scope — no clone needed).
struct PendingIngest<'a> {
    sync_key: String,
    task_id: String,
    title: String,
    updated: Option<String>,
    task: &'a Value,
}

/// Folded result of [`ingest_pending_buffered`]. Both fields are
/// order-independent, so the concurrent stage can accumulate into it
/// regardless of the order ingests complete.
#[derive(Default)]
struct BufferedIngestOutcome {
    /// `sync_key`s whose ingest succeeded — the caller marks each synced.
    synced_keys: Vec<String>,
    /// Number of tasks persisted (equals `synced_keys.len()`).
    persisted: usize,
}

/// Seam over "ingest one ClickUp task", so the bounded-concurrency driver
/// can be unit-tested with a fake that records peak in-flight calls
/// without a real memory tree or embedder.
#[async_trait]
trait TaskIngestor {
    async fn ingest(
        &self,
        task_id: &str,
        title: &str,
        updated: Option<&str>,
        task: &Value,
    ) -> anyhow::Result<usize>;
}

/// Production ingestor: routes into the memory-tree pipeline via
/// [`ingest_task_into_memory_tree`].
struct MemoryTreeIngestor<'c> {
    config: &'c Config,
    connection_id: &'c str,
}

#[async_trait]
impl TaskIngestor for MemoryTreeIngestor<'_> {
    async fn ingest(
        &self,
        task_id: &str,
        title: &str,
        updated: Option<&str>,
        task: &Value,
    ) -> anyhow::Result<usize> {
        ingest_task_into_memory_tree(
            self.config,
            self.connection_id,
            task_id,
            title,
            updated,
            task,
        )
        .await
    }
}

/// Pass 1 (pure, no I/O): scan one page of `tasks`, advance
/// `newest_updated`, skip already-synced items, and collect the tasks
/// still needing ingest. Returns the queued items and whether we crossed
/// the persistent cursor boundary (the signal to stop paginating this
/// workspace). All order-dependent decisions (cursor/timestamp) live here
/// — never in the concurrent stage.
fn select_pending<'a>(
    tasks: &'a [Value],
    state: &SyncState,
    newest_updated: &mut Option<String>,
) -> (Vec<PendingIngest<'a>>, bool) {
    let mut hit_cursor_boundary = false;
    let mut pending: Vec<PendingIngest> = Vec::new();
    for task in tasks {
        let Some(task_id) = extract_item_id(task, TASK_ID_PATHS) else {
            tracing::debug!("[composio:clickup] task missing ID, skipping");
            continue;
        };

        let updated = sync::extract_task_updated(task);

        // Track newest `date_updated` for cursor advancement.
        if let Some(ref ts) = updated {
            if newest_updated.as_ref().is_none_or(|existing| ts > existing) {
                *newest_updated = Some(ts.clone());
            }
        }

        // Composite (task_id, date_updated) key so a task edited *after*
        // its last sync is re-persisted. Same trick Notion uses for
        // `last_edited_time`.
        let sync_key = match &updated {
            Some(ts) => format!("{task_id}@{ts}"),
            None => task_id.clone(),
        };

        // Older than cursor AND already synced → caught up.
        if let (Some(ref cursor), Some(ref ts)) = (&state.cursor, &updated) {
            if ts <= cursor && state.is_synced(&sync_key) {
                hit_cursor_boundary = true;
                continue;
            }
        }

        if state.is_synced(&sync_key) {
            continue;
        }

        let title_text =
            sync::extract_task_name(task).unwrap_or_else(|| format!("ClickUp task {task_id}"));
        let title = format!("ClickUp: {title_text}");

        pending.push(PendingIngest {
            sync_key,
            task_id,
            title,
            updated,
            task,
        });
    }
    (pending, hit_cursor_boundary)
}

/// Pass 2: ingest the queued tasks with bounded concurrency. Overlaps the
/// per-item embedding RTT (`buffer_unordered`, up to `concurrency` in
/// flight) and folds results into an order-independent
/// [`BufferedIngestOutcome`]. Unordered is correct here: nothing
/// downstream depends on completion order — successes are keyed by
/// `sync_key`. A failed ingest is logged and skipped (parity with the
/// previous sequential `continue`); ClickUp advances its cursor regardless
/// of per-item failures.
async fn ingest_pending_buffered<I: TaskIngestor + Sync>(
    ingestor: &I,
    pending: Vec<PendingIngest<'_>>,
    workspace_id: &str,
    concurrency: usize,
) -> BufferedIngestOutcome {
    // Materialize the per-item futures into a Vec before `buffer_unordered`
    // so the spawned sync future keeps concrete lifetimes / `Send`.
    let ingest_futs = pending
        .into_iter()
        .map(|p| async move {
            let res = ingestor
                .ingest(&p.task_id, &p.title, p.updated.as_deref(), p.task)
                .await;
            (p.sync_key, p.task_id, res)
        })
        .collect::<Vec<_>>();

    let mut outcome = BufferedIngestOutcome::default();
    let mut ingest_stream = futures::stream::iter(ingest_futs).buffer_unordered(concurrency);
    while let Some((sync_key, task_id, res)) = ingest_stream.next().await {
        match res {
            Ok(_chunks_written) => {
                outcome.synced_keys.push(sync_key);
                outcome.persisted += 1;
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    workspace_id = %workspace_id,
                    error = %e,
                    "[composio:clickup] ingest failed (continuing)"
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
    /// `ingest` calls and can be told to fail one specific `task_id`. No
    /// memory tree or embedder involved — lets us assert the concurrency
    /// bound and overlap deterministically.
    struct CountingIngestor {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        fail_task: Option<String>,
    }

    impl CountingIngestor {
        fn new(fail_task: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                fail_task: fail_task.map(str::to_string),
            })
        }
    }

    #[async_trait]
    impl TaskIngestor for CountingIngestor {
        async fn ingest(
            &self,
            task_id: &str,
            _title: &str,
            _updated: Option<&str>,
            _task: &Value,
        ) -> anyhow::Result<usize> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            // Yield a few times so futures genuinely interleave and the
            // peak counter reflects real overlap, not accidental serial run.
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if self.fail_task.as_deref() == Some(task_id) {
                Err(anyhow::anyhow!("forced failure for {task_id}"))
            } else {
                Ok(1)
            }
        }
    }

    fn make_tasks(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({ "id": format!("t{i}") })).collect()
    }

    fn make_pending(tasks: &[Value]) -> Vec<PendingIngest<'_>> {
        tasks
            .iter()
            .enumerate()
            .map(|(i, task)| PendingIngest {
                sync_key: format!("k{i}"),
                task_id: format!("t{i}"),
                title: format!("ClickUp: task {i}"),
                updated: None,
                task,
            })
            .collect()
    }

    #[tokio::test]
    async fn ingest_pending_buffered_bounds_and_overlaps() {
        let ingestor = CountingIngestor::new(None);
        let tasks = make_tasks(20);
        let pending = make_pending(&tasks);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, "ws1", 8).await;

        assert_eq!(outcome.persisted, 20, "all tasks persisted");
        assert_eq!(outcome.synced_keys.len(), 20);

        let peak = ingestor.peak.load(Ordering::SeqCst);
        assert!(peak <= 8, "peak in-flight {peak} exceeded the bound of 8");
        assert!(peak >= 2, "peak in-flight {peak} shows no real overlap");
    }

    #[tokio::test]
    async fn ingest_pending_buffered_skips_failures_order_independent() {
        let ingestor = CountingIngestor::new(Some("t2"));
        let tasks = make_tasks(5);
        let pending = make_pending(&tasks);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, "ws1", 4).await;

        assert_eq!(outcome.persisted, 4, "the one failed ingest is not counted");
        assert_eq!(outcome.synced_keys.len(), 4);
        assert!(
            !outcome.synced_keys.contains(&"k2".to_string()),
            "the failed task's sync_key must not be marked synced"
        );
    }

    #[test]
    fn select_pending_tracks_newest_skips_synced_and_detects_boundary() {
        let mut state = SyncState::new("clickup", "conn1");
        state.cursor = Some("2026-04-15T00:00:00Z".to_string());
        // Task B is already synced and older than the cursor.
        state.mark_synced("b@2026-04-01T00:00:00Z");

        let tasks = vec![
            json!({ "id": "a", "date_updated": "2026-05-01T00:00:00Z" }),
            json!({ "id": "b", "date_updated": "2026-04-01T00:00:00Z" }),
            json!({ "date_updated": "2026-03-01T00:00:00Z" }), // no id → skipped
        ];

        let mut newest: Option<String> = None;
        let (pending, hit_boundary) = select_pending(&tasks, &state, &mut newest);

        assert_eq!(pending.len(), 1, "only the new task A is queued");
        assert_eq!(pending[0].task_id, "a");
        assert_eq!(pending[0].sync_key, "a@2026-05-01T00:00:00Z");
        assert!(
            hit_boundary,
            "older synced task B trips the cursor boundary"
        );
        assert_eq!(newest.as_deref(), Some("2026-05-01T00:00:00Z"));
    }
}
