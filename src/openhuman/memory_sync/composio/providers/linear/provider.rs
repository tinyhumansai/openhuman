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
use futures::StreamExt;
use serde_json::{json, Value};

use super::{ingest::ingest_issue_into_memory_tree, sync};
use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::sync_state::{extract_item_id, SyncState};
use crate::openhuman::memory_sync::composio::providers::{
    merge_extra, pick_str, resolve_sync_interval_secs, ComposioProvider, CuratedTool,
    NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason, TaskFetchFilter,
    TaskKind,
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

/// Max in-flight ingests per page. DB writes serialize anyway and the
/// cloud embedder has rate limits, so keep this small.
const INGEST_CONCURRENCY: usize = 8;

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
        Some(resolve_sync_interval_secs("linear", 30 * 60))
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

        // ctx.max_items: route through ItemCap — page ceiling, mid-page
        // per-item break, and post-page hard stop all share one source of truth.
        let mut cap = super::super::helpers::ItemCap::new(ctx.max_items);
        let effective_max_pages = cap.max_pages(page_size as u32, MAX_PAGES_PER_SYNC);
        if ctx.max_items.is_some() && effective_max_pages < MAX_PAGES_PER_SYNC {
            tracing::debug!(
                connection_id = %connection_id,
                max_items = ?ctx.max_items,
                effective_max_pages,
                "[composio:linear] [memory_sync] applying max_items page cap"
            );
        }

        // ctx.sync_depth_days: oldest allowed updatedAt for client-side skip.
        let oldest_allowed_time: Option<String> = ctx.sync_depth_days.map(|days| {
            let floor = chrono::Utc::now() - chrono::Duration::days(days as i64);
            let s = floor.to_rfc3339();
            tracing::debug!(
                connection_id = %connection_id,
                sync_depth_days = days,
                oldest_allowed = %s,
                "[composio:linear] [memory_sync] applying sync_depth_days floor"
            );
            s
        });

        let mut total_fetched: usize = 0;
        let mut total_persisted: usize = 0;
        let mut had_persist_failures = false;
        let mut newest_updated: Option<String> = None;
        let mut after_cursor: Option<String> = None;
        let mut hit_cursor_boundary = false;
        let mut hit_cap_boundary = false;

        for page_num in 0..effective_max_pages {
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

            // ── Per-item dedup + bounded-concurrency ingest ──────────
            let (mut pending, page_hit_boundary) =
                select_pending(&issues, &state, &mut newest_updated);
            if page_hit_boundary {
                hit_cursor_boundary = true;
            }

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
            let outcome = ingest_pending_buffered(&ingestor, pending, INGEST_CONCURRENCY).await;
            for key in &outcome.synced_keys {
                state.mark_synced(key);
            }
            total_persisted += outcome.persisted;
            cap.record(outcome.persisted);
            if outcome.had_failures {
                had_persist_failures = true;
            }

            // ctx.max_items precise cap: once the per-source cap is hit, stop paginating.
            if cap.is_reached() {
                hit_cap_boundary = true;
                break;
            }

            if hit_cursor_boundary {
                tracing::debug!(
                    page = page_num,
                    "[composio:linear] reached cursor boundary, stopping pagination"
                );
                break;
            }

            // ctx.max_items hard stop.
            if cap.is_reached() {
                tracing::debug!(
                    page = page_num,
                    total_persisted,
                    "[composio:linear] [memory_sync] max_items reached, stopping pagination"
                );
                hit_cap_boundary = true;
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
        // Hold the cursor on a cap-truncated pass so the next sync re-scans the unseen tail.
        if had_persist_failures {
            tracing::warn!(
                "[composio:linear] persist failures seen; keeping previous cursor for retry"
            );
        } else if hit_cap_boundary {
            tracing::warn!(
                hit_cap_boundary,
                "[composio:linear] holding cursor — cap-truncated pass; next sync will re-scan \
                 the unseen tail"
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
        kind: TaskKind::Generic,
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

/// One issue that passed dedupe in pass 1 and is queued for concurrent
/// ingest in pass 2. Borrows the raw issue `Value` out of the current
/// page's `issues` (same scope — no clone needed).
struct PendingIngest<'a> {
    sync_key: String,
    issue_id: String,
    title: String,
    updated: Option<String>,
    issue: &'a Value,
}

/// Folded result of [`ingest_pending_buffered`]. Every field is
/// order-independent, so the concurrent stage can accumulate into it
/// regardless of the order ingests complete.
#[derive(Default)]
struct BufferedIngestOutcome {
    /// `sync_key`s whose ingest succeeded — the caller marks each synced.
    synced_keys: Vec<String>,
    /// Number of issues persisted (equals `synced_keys.len()`).
    persisted: usize,
    /// Whether any per-item ingest failed (the caller holds the cursor).
    had_failures: bool,
}

/// Seam over "ingest one Linear issue", so the bounded-concurrency driver
/// can be unit-tested with a fake that records peak in-flight calls
/// without a real memory tree or embedder.
#[async_trait]
trait IssueIngestor {
    async fn ingest(
        &self,
        issue_id: &str,
        title: &str,
        updated: Option<&str>,
        issue: &Value,
    ) -> anyhow::Result<usize>;
}

/// Production ingestor: routes into the memory-tree pipeline via
/// [`ingest_issue_into_memory_tree`].
struct MemoryTreeIngestor<'c> {
    config: &'c Config,
    connection_id: &'c str,
}

#[async_trait]
impl IssueIngestor for MemoryTreeIngestor<'_> {
    async fn ingest(
        &self,
        issue_id: &str,
        title: &str,
        updated: Option<&str>,
        issue: &Value,
    ) -> anyhow::Result<usize> {
        ingest_issue_into_memory_tree(
            self.config,
            self.connection_id,
            issue_id,
            title,
            updated,
            issue,
        )
        .await
    }
}

/// Pass 1 (pure, no I/O): scan one page of `issues`, advance
/// `newest_updated`, skip already-synced items, and collect the issues
/// still needing ingest. Returns the queued items and whether we crossed
/// the persistent cursor boundary (the signal to stop paginating). All
/// order-dependent decisions (cursor/timestamp) live here — never in the
/// concurrent stage.
fn select_pending<'a>(
    issues: &'a [Value],
    state: &SyncState,
    newest_updated: &mut Option<String>,
) -> (Vec<PendingIngest<'a>>, bool) {
    let mut hit_cursor_boundary = false;
    let mut pending: Vec<PendingIngest> = Vec::new();
    for issue in issues {
        let Some(issue_id) = extract_item_id(issue, ISSUE_ID_PATHS) else {
            tracing::debug!("[composio:linear] issue missing ID, skipping");
            continue;
        };

        let updated = sync::extract_issue_updated(issue);

        // Track newest `updatedAt` for cursor advancement.
        if let Some(ref ts) = updated {
            if newest_updated.as_ref().is_none_or(|existing| ts > existing) {
                *newest_updated = Some(ts.clone());
            }
        }

        // Composite (issue_id, updatedAt) key so re-edited issues are
        // re-persisted on the next sync.
        let sync_key = match &updated {
            Some(ts) => format!("{issue_id}@{ts}"),
            None => issue_id.clone(),
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
            sync::extract_issue_title(issue).unwrap_or_else(|| format!("Linear issue {issue_id}"));
        let title = format!("Linear: {title_text}");

        pending.push(PendingIngest {
            sync_key,
            issue_id,
            title,
            updated,
            issue,
        });
    }
    (pending, hit_cursor_boundary)
}

/// Pass 2: ingest the queued issues with bounded concurrency. Overlaps
/// the per-item embedding RTT (`buffer_unordered`, up to `concurrency` in
/// flight) and folds results into an order-independent
/// [`BufferedIngestOutcome`]. Unordered is correct here: nothing
/// downstream depends on completion order — successes are keyed by
/// `sync_key`. A failed ingest is logged and skipped, tripping
/// `had_failures` so the caller holds the cursor (parity with the
/// previous sequential path).
async fn ingest_pending_buffered<I: IssueIngestor + Sync>(
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
                .ingest(&p.issue_id, &p.title, p.updated.as_deref(), p.issue)
                .await;
            (p.sync_key, p.issue_id, res)
        })
        .collect::<Vec<_>>();

    let mut outcome = BufferedIngestOutcome::default();
    let mut ingest_stream = futures::stream::iter(ingest_futs).buffer_unordered(concurrency);
    while let Some((sync_key, issue_id, res)) = ingest_stream.next().await {
        match res {
            Ok(_chunks_written) => {
                outcome.synced_keys.push(sync_key);
                outcome.persisted += 1;
            }
            Err(e) => {
                outcome.had_failures = true;
                tracing::warn!(
                    issue_id = %issue_id,
                    error = %e,
                    "[composio:linear] failed to ingest issue into memory_tree (continuing)"
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
    /// `ingest` calls and can be told to fail one specific `issue_id`. No
    /// memory tree or embedder involved — lets us assert the concurrency
    /// bound and overlap deterministically.
    struct CountingIngestor {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        fail_issue: Option<String>,
    }

    impl CountingIngestor {
        fn new(fail_issue: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                fail_issue: fail_issue.map(str::to_string),
            })
        }
    }

    #[async_trait]
    impl IssueIngestor for CountingIngestor {
        async fn ingest(
            &self,
            issue_id: &str,
            _title: &str,
            _updated: Option<&str>,
            _issue: &Value,
        ) -> anyhow::Result<usize> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            // Yield a few times so futures genuinely interleave and the
            // peak counter reflects real overlap, not accidental serial run.
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if self.fail_issue.as_deref() == Some(issue_id) {
                Err(anyhow::anyhow!("forced failure for {issue_id}"))
            } else {
                Ok(1)
            }
        }
    }

    fn make_issues(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({ "id": format!("i{i}") })).collect()
    }

    fn make_pending(issues: &[Value]) -> Vec<PendingIngest<'_>> {
        issues
            .iter()
            .enumerate()
            .map(|(i, issue)| PendingIngest {
                sync_key: format!("k{i}"),
                issue_id: format!("i{i}"),
                title: format!("Linear: issue {i}"),
                updated: None,
                issue,
            })
            .collect()
    }

    #[tokio::test]
    async fn ingest_pending_buffered_bounds_and_overlaps() {
        let ingestor = CountingIngestor::new(None);
        let issues = make_issues(20);
        let pending = make_pending(&issues);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, 8).await;

        assert_eq!(outcome.persisted, 20, "all issues persisted");
        assert_eq!(outcome.synced_keys.len(), 20);
        assert!(!outcome.had_failures);

        let peak = ingestor.peak.load(Ordering::SeqCst);
        assert!(peak <= 8, "peak in-flight {peak} exceeded the bound of 8");
        assert!(peak >= 2, "peak in-flight {peak} shows no real overlap");
    }

    #[tokio::test]
    async fn ingest_pending_buffered_skips_failures_order_independent() {
        let ingestor = CountingIngestor::new(Some("i2"));
        let issues = make_issues(5);
        let pending = make_pending(&issues);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, 4).await;

        assert_eq!(outcome.persisted, 4, "the one failed ingest is not counted");
        assert!(outcome.had_failures);
        assert_eq!(outcome.synced_keys.len(), 4);
        assert!(
            !outcome.synced_keys.contains(&"k2".to_string()),
            "the failed issue's sync_key must not be marked synced"
        );
    }

    #[test]
    fn select_pending_tracks_newest_skips_synced_and_detects_boundary() {
        let mut state = SyncState::new("linear", "conn1");
        state.cursor = Some("2026-04-15T00:00:00Z".to_string());
        // Issue B is already synced and older than the cursor.
        state.mark_synced("b@2026-04-01T00:00:00Z");

        let issues = vec![
            json!({ "id": "a", "updatedAt": "2026-05-01T00:00:00Z" }),
            json!({ "id": "b", "updatedAt": "2026-04-01T00:00:00Z" }),
            json!({ "updatedAt": "2026-03-01T00:00:00Z" }), // no id → skipped
        ];

        let mut newest: Option<String> = None;
        let (pending, hit_boundary) = select_pending(&issues, &state, &mut newest);

        assert_eq!(pending.len(), 1, "only the new issue A is queued");
        assert_eq!(pending[0].issue_id, "a");
        assert_eq!(pending[0].sync_key, "a@2026-05-01T00:00:00Z");
        assert!(
            hit_boundary,
            "older synced issue B trips the cursor boundary"
        );
        assert_eq!(newest.as_deref(), Some("2026-05-01T00:00:00Z"));
    }
}
