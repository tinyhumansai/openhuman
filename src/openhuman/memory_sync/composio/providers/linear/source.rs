//! Linear's [`IncrementalSource`] primitives.
//!
//! Linear rides the generic
//! [`crate::openhuman::memory_sync::composio::providers::orchestrator`]:
//! [`LinearProvider::sync`](super::provider::LinearProvider) delegates to
//! [`run_linear_sync`]. The orchestrator owns the control flow (budget,
//! pagination bound, dedup, the `sync_depth_days` window, the precise
//! `max_items` clamp, cursor advance/hold, state persistence); this module
//! supplies only the Linear-specific shapes.
//!
//! Linear is **flat but identity-scoped**: [`LinearSource::preamble`] resolves
//! the viewer id and returns a single [`SyncScope`] carrying it, then the
//! orchestrator pages straight through `LINEAR_LIST_LINEAR_ISSUES` filtered to
//! that assignee. Pagination is GraphQL cursor based (`after` / `endCursor`);
//! the depth window is applied client-side (RFC3339 `updatedAt`). Per-item
//! dedup is keyed by `{issue_id}@{updatedAt}` so an edited issue re-ingests.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use super::provider::{ACTION_LIST_ISSUES, ACTION_LIST_USERS, ISSUE_ID_PATHS};
use super::{ingest::ingest_issue_into_memory_tree, sync};
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

/// Maximum pages per sync pass before yielding.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Max in-flight ingests per page. DB writes serialize anyway and the cloud
/// embedder has rate limits, so keep this small.
const INGEST_CONCURRENCY: usize = 8;

/// Linear's [`IncrementalSource`].
pub(crate) struct LinearSource;

/// Entry point used by [`super::provider::LinearProvider::sync`].
pub(crate) async fn run_linear_sync(
    ctx: &ProviderContext,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    orchestrator::run_sync(&LinearSource, ctx, reason).await
}

#[async_trait]
impl IncrementalSource for LinearSource {
    fn toolkit(&self) -> &'static str {
        "linear"
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

    fn detail_noun(&self) -> &'static str {
        "issues"
    }

    /// Resolve the viewer id and return it as the single scope's id.
    async fn preamble(
        &self,
        ctx: &ProviderContext,
        state: &mut SyncState,
    ) -> Result<Vec<SyncScope>, String> {
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

        let viewer_id = sync::extract_viewer_id(&resp.data).ok_or_else(|| {
            "[composio:linear] LINEAR_LIST_LINEAR_USERS returned no viewer id".to_string()
        })?;
        Ok(vec![SyncScope::nested(viewer_id, "assignee:me")])
    }

    async fn fetch_page(
        &self,
        ctx: &ProviderContext,
        scope: &SyncScope,
        cursor: Option<&str>,
        reason: SyncReason,
        state: &mut SyncState,
    ) -> Result<PageFetch, String> {
        let mut args = json!({
            "assigneeId": &scope.id,
            "first": self.page_size(reason),
            "orderBy": "updatedAt",
        });
        if let Some(cursor) = cursor {
            args["after"] = json!(cursor);
        }

        let resp = ctx
            .execute(ACTION_LIST_ISSUES, Some(args))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_LIST_ISSUES}: {e:#}"))?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:linear] {ACTION_LIST_ISSUES}: {err}"));
        }

        Ok(PageFetch {
            items: sync::extract_issues(&resp.data),
            next: sync::extract_pagination_cursor(&resp.data),
        })
    }

    fn item_dedup_key(&self, item: &Value) -> Option<String> {
        let issue_id = extract_item_id(item, ISSUE_ID_PATHS)?;
        match sync::extract_issue_updated(item) {
            Some(updated) => Some(format!("{issue_id}@{updated}")),
            None => Some(issue_id),
        }
    }

    fn item_sort_ts(&self, item: &Value) -> Option<String> {
        sync::extract_issue_updated(item)
    }

    async fn ingest(
        &self,
        ctx: &ProviderContext,
        _scope: &SyncScope,
        _state: &mut SyncState,
        items: Vec<SyncItem>,
    ) -> IngestOutcome {
        let connection_id = ctx.connection_id.as_deref().unwrap_or("default");

        let pending: Vec<PendingIngest> = items
            .into_iter()
            .filter_map(|it| {
                let issue_id = extract_item_id(&it.raw, ISSUE_ID_PATHS)?;
                let title_text = sync::extract_issue_title(&it.raw)
                    .unwrap_or_else(|| format!("Linear issue {issue_id}"));
                Some(PendingIngest {
                    sync_key: it.dedup_key,
                    issue_id,
                    title: format!("Linear: {title_text}"),
                    updated: it.sort_ts,
                    issue: it.raw,
                })
            })
            .collect();

        let ingestor = MemoryTreeIngestor {
            config: ctx.config.as_ref(),
            connection_id,
        };
        let buffered = ingest_pending_buffered(&ingestor, pending, INGEST_CONCURRENCY).await;
        IngestOutcome {
            synced_keys: buffered.synced_keys,
            persisted: buffered.persisted,
            had_failures: buffered.had_failures,
        }
    }
}

/// One issue queued for concurrent ingest. Owns its raw issue `Value` (the
/// orchestrator handed ownership via [`SyncItem`]).
struct PendingIngest {
    sync_key: String,
    issue_id: String,
    title: String,
    updated: Option<String>,
    issue: Value,
}

/// Folded result of [`ingest_pending_buffered`]. Order-independent.
#[derive(Default)]
struct BufferedIngestOutcome {
    synced_keys: Vec<String>,
    persisted: usize,
    had_failures: bool,
}

/// Seam over "ingest one Linear issue", so the bounded-concurrency driver can
/// be unit-tested with a fake that records peak in-flight calls.
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

/// Production ingestor: routes into the memory-tree pipeline.
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

/// Ingest queued issues with bounded concurrency, folding into an
/// order-independent [`BufferedIngestOutcome`]. A failed ingest is logged and
/// skipped, tripping `had_failures` so the orchestrator holds the cursor.
async fn ingest_pending_buffered<I: IssueIngestor + Sync>(
    ingestor: &I,
    pending: Vec<PendingIngest>,
    concurrency: usize,
) -> BufferedIngestOutcome {
    let ingest_futs = pending
        .into_iter()
        .map(|p| async move {
            let res = ingestor
                .ingest(&p.issue_id, &p.title, p.updated.as_deref(), &p.issue)
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

    /// Fake ingestor: records peak concurrent in-flight `ingest` calls and can
    /// fail one specific `issue_id`. No memory tree or embedder involved.
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

    fn make_pending(n: usize) -> Vec<PendingIngest> {
        (0..n)
            .map(|i| PendingIngest {
                sync_key: format!("k{i}"),
                issue_id: format!("i{i}"),
                title: format!("Linear: issue {i}"),
                updated: None,
                issue: json!({ "id": format!("i{i}") }),
            })
            .collect()
    }

    #[tokio::test]
    async fn ingest_pending_buffered_bounds_and_overlaps() {
        let ingestor = CountingIngestor::new(None);
        let pending = make_pending(20);

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
        let pending = make_pending(5);

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
    fn item_dedup_key_composes_id_and_updated() {
        let with_update = json!({ "id": "i1", "updatedAt": "2026-05-01T00:00:00Z" });
        assert_eq!(
            LinearSource.item_dedup_key(&with_update).as_deref(),
            Some("i1@2026-05-01T00:00:00Z")
        );
        let no_update = json!({ "id": "i2" });
        assert_eq!(
            LinearSource.item_dedup_key(&no_update).as_deref(),
            Some("i2")
        );
        assert_eq!(
            LinearSource.item_dedup_key(&json!({ "updatedAt": "x" })),
            None
        );
    }
}
