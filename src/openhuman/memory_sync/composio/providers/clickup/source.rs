//! ClickUp's [`IncrementalSource`] primitives.
//!
//! ClickUp rides the generic
//! [`crate::openhuman::memory_sync::composio::providers::orchestrator`]:
//! [`ClickUpProvider::sync`](super::provider::ClickUpProvider) delegates to
//! [`run_clickup_sync`]. The orchestrator owns the control flow (budget,
//! pagination bound, dedup, the `sync_depth_days` window, the precise
//! `max_items` clamp, cursor advance/hold, state persistence); this module
//! supplies only the ClickUp-specific shapes.
//!
//! ClickUp is the first **nested** provider on the orchestrator:
//! [`ClickUpSource::preamble`] resolves the authorized user id + the visible
//! workspaces and returns **one [`SyncScope`] per workspace**, then the
//! orchestrator pages each workspace's `CLICKUP_GET_FILTERED_TEAM_TASKS`
//! (1-indexed `page`) scoped to that user as assignee. The shared user id is
//! resolved once in the preamble and stashed on the source ([`OnceLock`]) so
//! every per-workspace `fetch_page` can read it back.
//!
//! Two provider quirks the trait already accommodates:
//!   * **epoch-ms timestamps** — `date_updated` is a millisecond-epoch string,
//!     so [`ClickUpSource::depth_floor`] emits an epoch-ms floor (not RFC3339)
//!     for the lexicographic compare to be valid.
//!   * **advance-on-failure** — ClickUp advances its cursor even when a per-item
//!     ingest fails, so it overrides
//!     [`IncrementalSource::hold_cursor_on_ingest_failure`] to `false`.

use std::sync::OnceLock;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use super::provider::{
    ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES, ACTION_GET_AUTHORIZED_USER,
    ACTION_GET_FILTERED_TEAM_TASKS, TASK_ID_PATHS,
};
use super::{ingest::ingest_task_into_memory_tree, sync};
use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::orchestrator::{
    self, IncrementalSource, IngestOutcome, PageFetch, SyncItem, SyncScope,
};
use crate::openhuman::memory_sync::composio::providers::sync_state::{extract_item_id, SyncState};
use crate::openhuman::memory_sync::composio::providers::{
    ProviderContext, SyncOutcome, SyncReason,
};

/// Page size per API call on steady-state syncs.
const PAGE_SIZE: u32 = 50;

/// Larger initial-sync page size so the first backfill catches up faster.
const INITIAL_PAGE_SIZE: u32 = 100;

/// Maximum pages **per workspace** per sync pass.
const MAX_PAGES_PER_WORKSPACE: u32 = 20;

/// Max in-flight ingests per page. DB writes serialize anyway and the cloud
/// embedder has rate limits, so keep this small.
const INGEST_CONCURRENCY: usize = 8;

/// ClickUp's [`IncrementalSource`]. Holds the authorized user id resolved in
/// the preamble so every per-workspace `fetch_page` can scope to it.
pub(crate) struct ClickUpSource {
    user_id: OnceLock<String>,
}

impl ClickUpSource {
    fn new() -> Self {
        Self {
            user_id: OnceLock::new(),
        }
    }
}

/// Entry point used by [`super::provider::ClickUpProvider::sync`].
pub(crate) async fn run_clickup_sync(
    ctx: &ProviderContext,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    orchestrator::run_sync(&ClickUpSource::new(), ctx, reason).await
}

impl ClickUpSource {
    /// Look up (and budget-record) the authorized user's numeric id.
    async fn resolve_user_id(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<String, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:clickup] resolve_user_id via {ACTION_GET_AUTHORIZED_USER}"
        );

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

        let user_id = sync::extract_user_id(&resp.data).ok_or_else(|| {
            "[composio:clickup] CLICKUP_GET_AUTHORIZED_USER returned no user.id".to_string()
        })?;

        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:clickup] resolve_user_id complete"
        );
        Ok(user_id)
    }

    /// Look up (and budget-record) the workspace (team) ids visible to this
    /// connection.
    async fn resolve_workspaces(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<Vec<String>, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:clickup] resolve_workspaces via {ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES}"
        );

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

        let workspaces = sync::extract_workspace_ids(&resp.data);
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            count = workspaces.len(),
            "[composio:clickup] resolve_workspaces complete"
        );
        Ok(workspaces)
    }
}

#[async_trait]
impl IncrementalSource for ClickUpSource {
    fn toolkit(&self) -> &'static str {
        "clickup"
    }

    fn page_size(&self, reason: SyncReason) -> u32 {
        match reason {
            SyncReason::ConnectionCreated => INITIAL_PAGE_SIZE,
            _ => PAGE_SIZE,
        }
    }

    fn max_pages(&self) -> u32 {
        MAX_PAGES_PER_WORKSPACE
    }

    fn detail_noun(&self) -> &'static str {
        "tasks"
    }

    /// ClickUp advances its cursor even on per-item ingest failures.
    fn hold_cursor_on_ingest_failure(&self) -> bool {
        false
    }

    /// `date_updated` is a millisecond-epoch string, so the depth floor must be
    /// epoch-ms too (not RFC3339) for the lexicographic compare to hold.
    fn depth_floor(&self, days: u32) -> String {
        let floor = chrono::Utc::now() - chrono::Duration::days(days as i64);
        floor.timestamp_millis().to_string()
    }

    /// Resolve the user id (stashed for `fetch_page`) and return one scope per
    /// visible workspace. Mirrors the original's budget re-check: if the daily
    /// budget is exhausted right after the user-id probe, we skip the
    /// workspaces call (empty scopes → no-op outcome).
    async fn preamble(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<Vec<SyncScope>, String> {
        let user_id = self.resolve_user_id(ctx, state).await?;
        let _ = self.user_id.set(user_id);

        if state.budget_exhausted() {
            tracing::info!(
                "[composio:clickup] budget exhausted after user-id probe, skipping sync"
            );
            return Ok(vec![]);
        }

        let workspaces = self.resolve_workspaces(ctx, state).await?;
        Ok(workspaces
            .into_iter()
            .map(|ws| {
                let label = format!("workspace:{ws}");
                SyncScope::nested(ws, label)
            })
            .collect())
    }

    async fn fetch_page(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        cursor: Option<&str>,
        reason: SyncReason,
        state: &mut SyncState,
    ) -> Result<PageFetch, String> {
        let workspace_id = &scope.id;
        let user_id = self.user_id.get().cloned().unwrap_or_default();
        // ClickUp paginates by 0-indexed `page`; the orchestrator's opaque
        // cursor carries the next page number (`None` = first page).
        let page_num: u32 = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
        let page_size = self.page_size(reason);

        let args = json!({
            "team_id": workspace_id,
            "assignees": [user_id],
            "order_by": "updated",
            "reverse": true,
            "page": page_num,
            "page_size": page_size,
            "subtasks": true,
        });

        tracing::debug!(
            connection_id = ?ctx.connection_id,
            workspace_id = %workspace_id,
            page_num,
            page_size,
            "[composio:clickup] fetch_page via {ACTION_GET_FILTERED_TEAM_TASKS}"
        );

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
            return Err(format!(
                "[composio:clickup] {ACTION_GET_FILTERED_TEAM_TASKS} \
                 workspace={workspace_id} page={page_num}: {err}"
            ));
        }

        let tasks = sync::extract_tasks(&resp.data);
        // ClickUp signals the last page implicitly: a short page ends the
        // workspace.
        let next = if (tasks.len() as u32) < page_size {
            None
        } else {
            Some((page_num + 1).to_string())
        };

        tracing::debug!(
            connection_id = ?ctx.connection_id,
            workspace_id = %workspace_id,
            page_num,
            fetched = tasks.len(),
            has_next = next.is_some(),
            "[composio:clickup] fetch_page complete"
        );

        Ok(PageFetch { items: tasks, next })
    }

    fn item_dedup_key(&self, item: &Value) -> Option<String> {
        let task_id = extract_item_id(item, TASK_ID_PATHS)?;
        match sync::extract_task_updated(item) {
            Some(updated) => Some(format!("{task_id}@{updated}")),
            None => Some(task_id),
        }
    }

    fn item_sort_ts(&self, item: &Value) -> Option<String> {
        sync::extract_task_updated(item)
    }

    async fn ingest(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        _state: &mut SyncState,
        items: Vec<SyncItem>,
    ) -> IngestOutcome {
        let connection_id = ctx.connection_id.as_deref().unwrap_or("default");

        let pending: Vec<PendingIngest> = items
            .into_iter()
            .filter_map(|it| {
                let task_id = extract_item_id(&it.raw, TASK_ID_PATHS)?;
                let title_text = sync::extract_task_name(&it.raw)
                    .unwrap_or_else(|| format!("ClickUp task {task_id}"));
                Some(PendingIngest {
                    sync_key: it.dedup_key,
                    task_id,
                    title: format!("ClickUp: {title_text}"),
                    updated: it.sort_ts,
                    task: it.raw,
                })
            })
            .collect();

        let ingestor = MemoryTreeIngestor {
            config: ctx.config.as_ref(),
            connection_id,
        };
        let buffered =
            ingest_pending_buffered(&ingestor, pending, &scope.id, INGEST_CONCURRENCY).await;
        IngestOutcome {
            synced_keys: buffered.synced_keys,
            persisted: buffered.persisted,
            had_failures: buffered.had_failures,
        }
    }
}

/// One task queued for concurrent ingest. Owns its raw task `Value` (the
/// orchestrator handed ownership via [`SyncItem`]).
struct PendingIngest {
    sync_key: String,
    task_id: String,
    title: String,
    updated: Option<String>,
    task: Value,
}

/// Folded result of [`ingest_pending_buffered`]. Order-independent.
#[derive(Default)]
struct BufferedIngestOutcome {
    synced_keys: Vec<String>,
    persisted: usize,
    had_failures: bool,
}

/// Seam over "ingest one ClickUp task", so the bounded-concurrency driver can
/// be unit-tested with a fake that records peak in-flight calls.
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

/// Production ingestor: routes into the memory-tree pipeline.
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

/// Ingest queued tasks with bounded concurrency, folding into an
/// order-independent [`BufferedIngestOutcome`]. A failed ingest is logged and
/// skipped (ClickUp advances its cursor regardless, via
/// `hold_cursor_on_ingest_failure = false`).
async fn ingest_pending_buffered<I: TaskIngestor + Sync>(
    ingestor: &I,
    pending: Vec<PendingIngest>,
    workspace_id: &str,
    concurrency: usize,
) -> BufferedIngestOutcome {
    let ingest_futs = pending
        .into_iter()
        .map(|p| async move {
            let res = ingestor
                .ingest(&p.task_id, &p.title, p.updated.as_deref(), &p.task)
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
                outcome.had_failures = true;
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

    /// Fake ingestor: records peak concurrent in-flight `ingest` calls and can
    /// fail one specific `task_id`. No memory tree or embedder involved.
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

    fn make_pending(n: usize) -> Vec<PendingIngest> {
        (0..n)
            .map(|i| PendingIngest {
                sync_key: format!("k{i}"),
                task_id: format!("t{i}"),
                title: format!("ClickUp: task {i}"),
                updated: None,
                task: json!({ "id": format!("t{i}") }),
            })
            .collect()
    }

    #[tokio::test]
    async fn ingest_pending_buffered_bounds_and_overlaps() {
        let ingestor = CountingIngestor::new(None);
        let pending = make_pending(20);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, "ws1", 8).await;

        assert_eq!(outcome.persisted, 20, "all tasks persisted");
        assert_eq!(outcome.synced_keys.len(), 20);
        assert!(!outcome.had_failures);

        let peak = ingestor.peak.load(Ordering::SeqCst);
        assert!(peak <= 8, "peak in-flight {peak} exceeded the bound of 8");
        assert!(peak >= 2, "peak in-flight {peak} shows no real overlap");
    }

    #[tokio::test]
    async fn ingest_pending_buffered_skips_failures_order_independent() {
        let ingestor = CountingIngestor::new(Some("t2"));
        let pending = make_pending(5);

        let outcome = ingest_pending_buffered(ingestor.as_ref(), pending, "ws1", 4).await;

        assert_eq!(outcome.persisted, 4, "the one failed ingest is not counted");
        assert!(outcome.had_failures);
        assert_eq!(outcome.synced_keys.len(), 4);
        assert!(!outcome.synced_keys.contains(&"k2".to_string()));
    }

    #[test]
    fn item_dedup_key_composes_id_and_updated() {
        let with_update = json!({ "id": "t1", "date_updated": "1780000000000" });
        assert_eq!(
            ClickUpSource::new().item_dedup_key(&with_update).as_deref(),
            Some("t1@1780000000000")
        );
        let no_update = json!({ "id": "t2" });
        assert_eq!(
            ClickUpSource::new().item_dedup_key(&no_update).as_deref(),
            Some("t2")
        );
        assert_eq!(
            ClickUpSource::new().item_dedup_key(&json!({ "date_updated": "x" })),
            None
        );
    }

    #[test]
    fn depth_floor_is_epoch_millis() {
        let floor = ClickUpSource::new().depth_floor(7);
        // Epoch-ms string: all digits, 13 chars in the 2020s+.
        assert!(floor.chars().all(|c| c.is_ascii_digit()), "got {floor}");
        assert!(
            floor.len() >= 13,
            "epoch-ms should be >=13 digits, got {floor}"
        );
    }
}
