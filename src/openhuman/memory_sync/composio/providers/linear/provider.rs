//! Linear provider — incremental sync of issues assigned to the
//! authenticated user, with per-issue memory_tree ingest.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Resolve the viewer ID via `LINEAR_LIST_LINEAR_USERS { isMe: true }`.
//!   4. Page through `LINEAR_LIST_LINEAR_ISSUES` filtered to the viewer as
//!      assignee, ordered by `updatedAt` descending. Stop early once we hit
//!      issues older than the cursor or a page without a next-page cursor.
//!   5. For each issue, ingest into memory_tree if it's new *or* edited
//!      since the last sync.
//!   6. Advance the cursor to the newest `updatedAt` seen and save.
//!
//! Privacy posture: we only pull issues the user is assigned to, never
//! the whole workspace's issue graph. This mirrors the
//! "fetch-what-the-user-sees" model `gmail` / `notion` already follow
//! and avoids accidentally ingesting other teammates' private issues.

use async_trait::async_trait;
use serde_json::json;

use super::{ingest::ingest_issue_into_memory_tree, sync};
use crate::openhuman::memory_sync::composio::providers::sync_state::{extract_item_id, SyncState};
use crate::openhuman::memory_sync::composio::providers::{
    merge_extra, pick_str, ComposioProvider, CuratedTool, NormalizedTask, ProviderContext,
    ProviderUserProfile, SyncOutcome, SyncReason, TaskFetchFilter,
};

const ACTION_LIST_USERS: &str = "LINEAR_LIST_LINEAR_USERS";
const ACTION_LIST_ISSUES: &str = "LINEAR_LIST_LINEAR_ISSUES";

/// Page size per API call. We use a small window on steady-state syncs
/// to keep response sizes bounded.
const PAGE_SIZE: u64 = 50;

/// Larger initial-sync page size so the first backfill catches up faster.
const INITIAL_PAGE_SIZE: u64 = 100;

/// Maximum pages per sync pass before yielding. Caps initial backfill
/// churn — anything beyond this rolls over to the next sync interval.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Paths for extracting a Linear issue's unique ID.
const ISSUE_ID_PATHS: &[&str] = &["id", "data.id", "identifier", "data.identifier"];

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
        // 30 minutes — same cadence as ClickUp/Notion. Linear issues change
        // more slowly than chat but faster than email.
        Some(30 * 60)
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:linear] fetch_user_profile via {ACTION_LIST_USERS}"
        );

        let resp = ctx
            .execute(ACTION_LIST_USERS, Some(json!({ "isMe": true })))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_LIST_USERS} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:linear] {ACTION_LIST_USERS}: {err}"));
        }

        let data = &resp.data;
        let viewer = sync::extract_viewer(data);
        let viewer_ref = viewer.as_ref().unwrap_or(data);

        let display_name = pick_str(viewer_ref, &["name", "data.name", "displayName"]);
        let email = pick_str(viewer_ref, &["email", "data.email"]);
        let username = pick_str(viewer_ref, &["id", "data.id"]);
        let avatar_url = pick_str(viewer_ref, &["avatarUrl", "data.avatarUrl"]);
        let profile_url = pick_str(viewer_ref, &["url", "data.url"]);

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

        // ── Step 3: resolve the authenticated user's ID ─────────────
        let viewer_id = match self.resolve_viewer_id(ctx, &mut state).await {
            Ok(id) => id,
            Err(e) => {
                let _ = state.save(&memory).await;
                return Err(e);
            }
        };

        // Re-check budget after the viewer-id probe.
        if state.budget_exhausted() {
            tracing::info!(
                connection_id = %connection_id,
                "[composio:linear] budget exhausted after viewer-id probe, skipping sync"
            );
            state.save(&memory).await?;
            return Ok(SyncOutcome {
                toolkit: "linear".to_string(),
                connection_id: Some(connection_id),
                reason: reason.as_str().to_string(),
                items_ingested: 0,
                started_at_ms,
                finished_at_ms: sync::now_ms(),
                summary: "linear sync skipped: daily budget exhausted after viewer-id probe"
                    .to_string(),
                details: json!({ "budget_exhausted": true, "viewer_id_resolved": true }),
            });
        }

        // ── Step 4: paginated incremental fetch ──────────────────────
        let page_size = match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        };

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut had_persist_failures = false;
        let mut newest_updated: Option<String> = None;
        let mut after_cursor: Option<String> = None;
        let mut hit_cursor_boundary = false;

        for page_num in 0..MAX_PAGES_PER_SYNC {
            if state.budget_exhausted() {
                tracing::info!(
                    page = page_num,
                    "[composio:linear] budget exhausted mid-sync, stopping pagination"
                );
                break;
            }

            let mut args = json!({
                "assigneeId": &viewer_id,
                "first": page_size,
                "orderBy": "updatedAt",
            });

            if let Some(ref cursor) = after_cursor {
                args["after"] = json!(cursor);
            }

            let resp = ctx
                .execute(ACTION_LIST_ISSUES, Some(args))
                .await
                .map_err(|e| {
                    format!("[composio:linear] {ACTION_LIST_ISSUES} page={page_num}: {e:#}")
                })?;

            state.record_requests(1);

            if !resp.successful {
                let err = resp
                    .error
                    .clone()
                    .unwrap_or_else(|| "provider reported failure".to_string());
                let _ = state.save(&memory).await;
                return Err(format!(
                    "[composio:linear] {ACTION_LIST_ISSUES} page={page_num}: {err}"
                ));
            }

            let issues = sync::extract_issues(&resp.data);
            total_fetched += issues.len();

            if issues.is_empty() {
                tracing::debug!(
                    page = page_num,
                    "[composio:linear] empty page, stopping pagination"
                );
                break;
            }

            // ── Per-item dedup + persist ─────────────────────────────
            for issue in &issues {
                let Some(issue_id) = extract_item_id(issue, ISSUE_ID_PATHS) else {
                    tracing::debug!("[composio:linear] issue missing ID, skipping");
                    continue;
                };

                let updated = sync::extract_issue_updated(issue);

                // Track newest `updatedAt` for cursor advancement.
                if let Some(ref ts) = updated {
                    if newest_updated.as_ref().is_none_or(|existing| ts > existing) {
                        newest_updated = Some(ts.clone());
                    }
                }

                // Composite (issue_id, updatedAt) key so re-edited
                // issues are re-persisted on the next sync.
                let sync_key = match &updated {
                    Some(ts) => format!("{issue_id}@{ts}"),
                    None => issue_id.clone(),
                };

                // If `updatedAt` is at or older than our cursor *and*
                // we already synced this key, the rest of the page is
                // by definition older — stop early.
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
                let title = format!("Linear: {title_text}");

                match ingest_issue_into_memory_tree(
                    &ctx.config,
                    &connection_id,
                    &issue_id,
                    &title,
                    updated.as_deref(),
                    issue,
                )
                .await
                {
                    Ok(_) => {
                        state.mark_synced(&sync_key);
                        total_persisted += 1;
                    }
                    Err(e) => {
                        had_persist_failures = true;
                        tracing::warn!(
                            issue_id = %issue_id,
                            error = %e,
                            "[composio:linear] failed to ingest issue into memory_tree (continuing)"
                        );
                    }
                }
            }

            if hit_cursor_boundary {
                tracing::debug!(
                    page = page_num,
                    "[composio:linear] reached cursor boundary, stopping pagination"
                );
                break;
            }

            // Advance to the next page using Linear's cursor-based pagination.
            match sync::extract_pagination_cursor(&resp.data) {
                Some(next_cursor) => {
                    after_cursor = Some(next_cursor);
                }
                None => {
                    tracing::debug!(
                        page = page_num,
                        "[composio:linear] no next page cursor, end of results"
                    );
                    break;
                }
            }
        }

        // ── Step 5: advance cursor and save state ────────────────────
        if had_persist_failures {
            tracing::warn!(
                "[composio:linear] persist failures seen; keeping previous cursor for retry"
            );
        } else if let Some(new_cursor) = newest_updated {
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
            "[composio:linear] fetch_tasks"
        );

        let mut args = json!({
            "first": max.min(100) as u64,
            "orderBy": "updatedAt",
        });
        if filter.assignee_is_me {
            let resp = ctx
                .execute(ACTION_LIST_USERS, Some(json!({ "isMe": true })))
                .await
                .map_err(|e| format!("[composio:linear] {ACTION_LIST_USERS}: {e:#}"))?;
            // Fail closed: a failed viewer lookup must not silently widen
            // the query beyond "assigned to me".
            if !resp.successful {
                return Err(format!(
                    "[composio:linear] {ACTION_LIST_USERS}: {}",
                    resp.error.unwrap_or_else(|| "provider failure".into())
                ));
            }
            let viewer_id = sync::extract_viewer_id(&resp.data).ok_or_else(|| {
                "[composio:linear] LINEAR_LIST_LINEAR_USERS returned no viewer id".to_string()
            })?;
            args["assigneeId"] = json!(viewer_id);
        }
        if let Some(team) = filter
            .team_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            args["teamId"] = json!(team);
        }
        merge_extra(&mut args, &filter.extra);

        let resp = ctx
            .execute(ACTION_LIST_ISSUES, Some(args))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_LIST_ISSUES}: {e:#}"))?;
        if !resp.successful {
            return Err(format!(
                "[composio:linear] {ACTION_LIST_ISSUES}: {}",
                resp.error.unwrap_or_else(|| "provider failure".into())
            ));
        }

        let want_state = filter
            .state
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());

        let mut out: Vec<NormalizedTask> = Vec::new();
        for issue in sync::extract_issues(&resp.data) {
            if out.len() >= max {
                break;
            }
            let Some(nt) = normalize_linear_issue(&issue) else {
                continue;
            };
            if let Some(ref want) = want_state {
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
        tracing::debug!(count = out.len(), "[composio:linear] fetch_tasks complete");
        Ok(out)
    }
}

/// Map a raw Linear issue payload into a [`NormalizedTask`].
fn normalize_linear_issue(issue: &serde_json::Value) -> Option<NormalizedTask> {
    let external_id = extract_item_id(issue, ISSUE_ID_PATHS)?;
    let title =
        sync::extract_issue_title(issue).unwrap_or_else(|| format!("Linear issue {external_id}"));
    Some(NormalizedTask {
        external_id,
        source_id: String::new(),
        provider: "linear".to_string(),
        title,
        body: pick_str(issue, &["description", "data.description"]),
        url: pick_str(issue, &["url", "data.url"]),
        status: pick_str(issue, &["state.name", "data.state.name", "state.type"]),
        assignee: pick_str(issue, &["assignee.name", "data.assignee.name"]),
        due: pick_str(issue, &["dueDate", "data.dueDate"]),
        labels: extract_linear_labels(issue),
        priority: pick_str(issue, &["priorityLabel", "data.priorityLabel"]),
        updated_at: sync::extract_issue_updated(issue),
        raw: issue.clone(),
    })
}

/// Extract label names from a Linear issue (`labels.nodes[].name`).
fn extract_linear_labels(issue: &serde_json::Value) -> Vec<String> {
    let arr = issue
        .get("labels")
        .or_else(|| issue.get("data").and_then(|d| d.get("labels")))
        .and_then(|l| l.get("nodes"))
        .and_then(|v| v.as_array());
    match arr {
        Some(items) => items
            .iter()
            .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    }
}

impl LinearProvider {
    /// Look up (and budget-record) the authenticated viewer's ID.
    ///
    /// The ID is stable for the connection's lifetime. We re-fetch on
    /// every sync rather than caching it in `SyncState` because (a) the
    /// call is cheap, (b) it implicitly validates that the OAuth
    /// connection is still good before we start paginating.
    async fn resolve_viewer_id(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<String, String> {
        let resp = ctx
            .execute(ACTION_LIST_USERS, Some(json!({ "isMe": true })))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_LIST_USERS} failed: {e:#}"))?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:linear] {ACTION_LIST_USERS}: {err}"));
        }

        sync::extract_viewer_id(&resp.data).ok_or_else(|| {
            "[composio:linear] LINEAR_LIST_LINEAR_USERS returned no viewer id".to_string()
        })
    }
}
