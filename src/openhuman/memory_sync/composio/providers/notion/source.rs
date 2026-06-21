//! Notion's [`IncrementalSource`] primitives.
//!
//! Notion is the first provider converted to the generic
//! [`crate::openhuman::memory_sync::composio::providers::orchestrator`]:
//! [`NotionProvider::sync`](super::provider::NotionProvider) delegates to
//! [`run_notion_sync`], which runs [`NotionSource`] through
//! [`orchestrator::run_sync`]. The orchestrator owns the control flow (budget,
//! pagination bound, dedup, the `sync_depth_days` window, the precise
//! `max_items` clamp, cursor advance/hold, state persistence); this module
//! supplies only the Notion-specific shapes.
//!
//! Notion is a **flat** provider — [`NotionSource::preamble`] returns a single
//! [`SyncScope::flat`] and the orchestrator pages straight through
//! `NOTION_FETCH_DATA`. Per-item dedup is keyed by `{page_id}@{edited_time}` so
//! an *edited* page (its `last_edited_time` advances) is re-ingested, exactly as
//! before. The rendered page **body** is fetched per page via
//! `NOTION_GET_PAGE_MARKDOWN` inside [`NotionSource::ingest`] (budget-counted),
//! then handed to the memory-tree pipeline.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use super::ingest::ingest_page_into_memory_tree;
use super::provider::{ACTION_FETCH_DATA, PAGE_EDITED_PATHS, PAGE_ID_PATHS};
use super::sync;
use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::orchestrator::{
    self, IncrementalSource, IngestOutcome, PageFetch, SyncItem, SyncScope,
};
use crate::openhuman::memory_sync::composio::providers::sync_state::{extract_item_id, SyncState};
use crate::openhuman::memory_sync::composio::providers::{
    ProviderContext, SyncOutcome, SyncReason,
};

/// Per-page action that returns the page's rendered **body** as markdown
/// (paragraphs, headings, lists, body tables). `NOTION_FETCH_DATA` only returns
/// metadata + properties; this fills in the actual document content for
/// free-form pages. One request per page (budget-counted).
pub(crate) const ACTION_GET_PAGE_MARKDOWN: &str = "NOTION_GET_PAGE_MARKDOWN";

/// Page size per API call.
const PAGE_SIZE: u32 = 25;

/// Larger page size for initial sync after OAuth.
const INITIAL_PAGE_SIZE: u32 = 50;

/// Maximum pages per sync pass.
const MAX_PAGES_PER_SYNC: u32 = 20;

/// Max in-flight ingests per page. DB writes serialize anyway and the
/// cloud embedder has rate limits, so keep this small.
const INGEST_CONCURRENCY: usize = 8;

/// Max in-flight `GET_PAGE_MARKDOWN` body fetches per page. Kept small to
/// respect Notion/Composio rate limits while still overlapping the per-request
/// round-trips instead of paying them strictly serially.
const BODY_FETCH_CONCURRENCY: usize = 5;

/// How many pending pages we may fetch bodies for, given the remaining daily
/// request budget. Pure so it can be unit-tested: it reproduces the old serial
/// "check `budget_exhausted()` before each fetch, break when it hits zero"
/// behaviour as a single up-front clamp (one body fetch == one request).
fn body_fetch_count(pending_len: usize, budget_remaining: u32) -> usize {
    pending_len.min(budget_remaining as usize)
}

/// Notion's [`IncrementalSource`] — supplies only the provider-specific shapes;
/// the orchestrator owns everything else.
pub(crate) struct NotionSource;

/// Entry point used by [`super::provider::NotionProvider::sync`].
pub(crate) async fn run_notion_sync(
    ctx: &ProviderContext,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    orchestrator::run_sync(&NotionSource, ctx, reason).await
}

#[async_trait]
impl IncrementalSource for NotionSource {
    fn toolkit(&self) -> &'static str {
        "notion"
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

    /// Notion is flat — one implicit scope, no identity resolution needed here
    /// (the user profile is fetched separately by `on_connection_created`).
    async fn preamble(
        &self,
        _ctx: &ProviderContext,
        _state: &mut SyncState,
    ) -> Result<Vec<SyncScope>, String> {
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
        let mut args = json!({
            "page_size": self.page_size(reason),
            "filter": { "value": "page", "property": "object" },
            "sort": { "direction": "descending", "timestamp": "last_edited_time" }
        });
        if let Some(cursor) = cursor {
            args["start_cursor"] = json!(cursor);
        }

        // A transport error never reached Composio — `?` returns before we
        // record. A completed round-trip is recorded against the daily budget
        // *before* we surface a provider-reported failure, so a broken
        // connection cannot make unlimited billable failed page calls.
        let resp = ctx
            .execute(ACTION_FETCH_DATA, Some(args))
            .await
            .map_err(|e| format!("[composio:notion] {ACTION_FETCH_DATA}: {e:#}"))?;
        state.record_requests(1);

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:notion] {ACTION_FETCH_DATA}: {err}"));
        }

        Ok(PageFetch {
            items: sync::extract_results(&resp.data),
            next: sync::extract_notion_cursor(&resp.data),
        })
    }

    /// Dedup key is `{page_id}@{last_edited_time}` (or the bare id when no
    /// edited time) so an edited page re-ingests while an unchanged one is
    /// skipped.
    fn item_dedup_key(&self, item: &Value) -> Option<String> {
        let page_id = extract_item_id(item, PAGE_ID_PATHS)?;
        match extract_item_id(item, PAGE_EDITED_PATHS) {
            Some(edited) => Some(format!("{page_id}@{edited}")),
            None => Some(page_id),
        }
    }

    fn item_sort_ts(&self, item: &Value) -> Option<String> {
        extract_item_id(item, PAGE_EDITED_PATHS)
    }

    async fn ingest(
        &self,
        ctx: &ProviderContext,
        _scope: &SyncScope,
        state: &mut SyncState,
        items: Vec<SyncItem>,
    ) -> IngestOutcome {
        let connection_id = ctx.connection_id.as_deref().unwrap_or("default");

        // Build the per-page ingest queue, re-extracting id/title from each raw
        // page. `sync_key`/`edited_time` come straight from the orchestrator's
        // dedup/sort decision so they stay consistent with `mark_synced`.
        let mut pending: Vec<PendingIngest> = items
            .into_iter()
            .filter_map(|it| {
                let page_id = extract_item_id(&it.raw, PAGE_ID_PATHS)?;
                let title_text = sync::extract_page_title(&it.raw)
                    .unwrap_or_else(|| format!("Notion page {page_id}"));
                Some(PendingIngest {
                    sync_key: it.dedup_key,
                    page_id,
                    title: format!("Notion: {title_text}"),
                    edited_time: it.sort_ts,
                    page: it.raw,
                    markdown_body: None,
                })
            })
            .collect();

        // ── Per-page BODY markdown fetch (bounded concurrency) ──────────
        // `NOTION_FETCH_DATA` returned metadata + properties only. Pull the
        // rendered page body per page so free-form documents ingest as real
        // multi-chunk content. One request per page — budget counted. We
        // pre-clamp the number of bodies to the remaining daily budget
        // (reproducing the old serial "check before each fetch, break at zero"
        // semantics as a single up-front decision) and fire the fetches
        // `BODY_FETCH_CONCURRENCY`-at-a-time; `buffered` preserves input order
        // so result[i] maps back to pending[i].
        let fetch_count = body_fetch_count(pending.len(), state.budget_remaining());
        if fetch_count < pending.len() {
            tracing::info!(
                fetch_count,
                pending = pending.len(),
                "[composio:notion] daily budget caps body fetch — \
                 remaining pages ingest metadata-only"
            );
        }
        if fetch_count > 0 {
            // Snapshot the page ids so the fetch futures don't borrow `pending`
            // (we write results back into it afterwards).
            let page_ids: Vec<String> = pending[..fetch_count]
                .iter()
                .map(|p| p.page_id.clone())
                .collect();

            let body_futs: Vec<_> = page_ids
                .into_iter()
                .map(|page_id| {
                    let ctx = &ctx;
                    async move {
                        match ctx
                            .execute(
                                ACTION_GET_PAGE_MARKDOWN,
                                Some(json!({ "page_id": &page_id })),
                            )
                            .await
                        {
                            Ok(resp) if resp.successful => {
                                let markdown = sync::extract_page_markdown(&resp.data);
                                if markdown.is_none() {
                                    tracing::warn!(
                                        page_id = %page_id,
                                        "[composio:notion] GET_PAGE_MARKDOWN returned no \
                                         markdown field — metadata-only fallback"
                                    );
                                }
                                markdown
                            }
                            Ok(resp) => {
                                tracing::warn!(
                                    page_id = %page_id,
                                    error = ?resp.error,
                                    "[composio:notion] GET_PAGE_MARKDOWN failed — \
                                     metadata-only fallback"
                                );
                                None
                            }
                            Err(e) => {
                                tracing::warn!(
                                    page_id = %page_id,
                                    error = %e,
                                    "[composio:notion] GET_PAGE_MARKDOWN execute error — \
                                     metadata-only fallback"
                                );
                                None
                            }
                        }
                    }
                })
                .collect();

            let bodies: Vec<Option<String>> = futures::stream::iter(body_futs)
                .buffered(BODY_FETCH_CONCURRENCY)
                .collect()
                .await;

            // Count every request we fired (success or failure), matching the
            // old per-call `record_requests(1)`.
            state.record_requests(fetch_count as u32);

            for (p, body) in pending.iter_mut().zip(bodies.into_iter()) {
                p.markdown_body = body;
            }
        }

        // ── Ingest queued pages (bounded concurrency) ───────────────────
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

/// One page queued for concurrent ingest. Owns its raw page `Value` (the
/// orchestrator handed ownership via [`SyncItem`]).
struct PendingIngest {
    sync_key: String,
    page_id: String,
    title: String,
    edited_time: Option<String>,
    page: Value,
    /// Rendered page body (markdown) fetched per-page via
    /// `NOTION_GET_PAGE_MARKDOWN`. `None` when the body fetch was skipped
    /// (budget) or failed — ingest falls back to the metadata-only body.
    markdown_body: Option<String>,
}

/// Folded result of [`ingest_pending_buffered`]. Every field is
/// order-independent so the concurrent stage can accumulate regardless of the
/// order ingests complete.
#[derive(Default)]
struct BufferedIngestOutcome {
    /// `sync_key`s whose ingest succeeded — the orchestrator marks each synced.
    synced_keys: Vec<String>,
    /// Number of pages persisted (equals `synced_keys.len()`).
    persisted: usize,
    /// Whether any per-item ingest failed (the orchestrator holds the cursor).
    had_failures: bool,
}

/// Seam over "ingest one Notion page", so the bounded-concurrency driver can be
/// unit-tested with a fake that records peak in-flight calls without a real
/// memory tree or embedder.
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

/// Ingest the queued pages with bounded concurrency. Overlaps the per-item
/// embedding RTT (`buffer_unordered`, up to `concurrency` in flight) and folds
/// results into an order-independent [`BufferedIngestOutcome`]. Unordered is
/// correct here: nothing downstream depends on completion order — successes are
/// keyed by `sync_key`.
async fn ingest_pending_buffered<I: PageIngestor + Sync>(
    ingestor: &I,
    pending: Vec<PendingIngest>,
    concurrency: usize,
) -> BufferedIngestOutcome {
    let ingest_futs = pending
        .into_iter()
        .map(|p| async move {
            let res = ingestor
                .ingest(
                    &p.page_id,
                    &p.title,
                    p.edited_time.as_deref(),
                    &p.page,
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
mod body_fetch_count_tests {
    use super::body_fetch_count;

    #[test]
    fn clamps_to_remaining_budget_when_budget_is_the_limit() {
        // 10 pending pages but only 3 requests left today → fetch 3.
        assert_eq!(body_fetch_count(10, 3), 3);
    }

    #[test]
    fn fetches_all_when_budget_exceeds_pending() {
        assert_eq!(body_fetch_count(4, 100), 4);
    }

    #[test]
    fn zero_budget_fetches_nothing() {
        // Mirrors the old serial "budget_exhausted() before first fetch" break.
        assert_eq!(body_fetch_count(7, 0), 0);
    }

    #[test]
    fn zero_pending_fetches_nothing() {
        assert_eq!(body_fetch_count(0, 50), 0);
    }
}

#[cfg(test)]
mod buffered_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Fake ingestor: records the peak number of concurrent in-flight `ingest`
    /// calls and can be told to fail one specific `page_id`. No memory tree or
    /// embedder involved — lets us assert the concurrency bound and overlap
    /// deterministically.
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

    fn make_pending(n: usize) -> Vec<PendingIngest> {
        (0..n)
            .map(|i| PendingIngest {
                sync_key: format!("k{i}"),
                page_id: format!("p{i}"),
                title: format!("Notion: page {i}"),
                edited_time: None,
                page: json!({ "id": format!("p{i}") }),
                markdown_body: None,
            })
            .collect()
    }

    #[tokio::test]
    async fn ingest_pending_buffered_bounds_and_overlaps() {
        let ingestor = CountingIngestor::new(None);
        let pending = make_pending(20);

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
        let pending = make_pending(5);

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
    fn item_dedup_key_composes_id_and_edited_time() {
        let with_edit = json!({ "id": "p1", "last_edited_time": "2026-05-01T00:00:00Z" });
        assert_eq!(
            NotionSource.item_dedup_key(&with_edit).as_deref(),
            Some("p1@2026-05-01T00:00:00Z")
        );
        // No edited time → bare id.
        let no_edit = json!({ "id": "p2" });
        assert_eq!(NotionSource.item_dedup_key(&no_edit).as_deref(), Some("p2"));
        // No id at all → dropped.
        assert_eq!(
            NotionSource.item_dedup_key(&json!({ "last_edited_time": "x" })),
            None
        );
    }
}
