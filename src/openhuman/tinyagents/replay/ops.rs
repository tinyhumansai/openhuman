//! Read-only business logic for the agent replay/status RPC surface
//! (workstream 05.x). Every function here is a *reader* over the C4 durable
//! journal + status seams built in
//! [`crate::openhuman::tinyagents::journal`] — it opens the same
//! `{workspace}/tinyagents_store/{kv,journal}` stores and never writes, mutates,
//! or bypasses any security/approval/sandbox gate.
//!
//! The controller layer ([`super::schemas`]) resolves the configured workspace
//! and delegates here; these functions take an explicit `workspace` path so they
//! are unit-testable against a temp store (mirroring the `journal.rs` tests).

use std::path::Path;

use tinyagents::harness::events::HarnessRunStatus;
use tinyagents::harness::ids::ExecutionStatus;
use tinyagents::harness::observability::{
    AgentObservation, HarnessEventJournal, HarnessStatusStore, StoreEventJournal,
};

use crate::openhuman::session_import::ops::open_session_stores;
use crate::openhuman::tinyagents::journal::FileStatusStore;

/// Default page size for [`read_run_events_page`] when the caller omits `limit`.
pub(crate) const DEFAULT_EVENTS_LIMIT: u64 = 200;

/// Hard cap on a single replay page so one RPC can never fan a whole run into a
/// single response.
pub(crate) const MAX_EVENTS_LIMIT: u64 = 1000;

/// One page of a run's durable event stream.
///
/// `events` are [`AgentObservation`]s (the crate's durable observability
/// envelope — `event_id` / `run_id` / lineage / `offset` / `ts_ms` / typed
/// `event`) in ascending `offset` order. `next_offset` is the `offset` to pass
/// back to fetch the following page, or `None` once the stream is drained.
pub(crate) struct RunEventsPage {
    pub events: Vec<AgentObservation>,
    pub next_offset: Option<u64>,
}

/// Paged late-attach replay reader over the durable journal.
///
/// Returns up to `limit` observations for `run_id` whose stream offset is
/// `>= offset`, in order, plus a `next_offset` cursor (`None` when the last page
/// drained the stream). Backed by the C4 journal seam
/// ([`StoreEventJournal::read_from`], the same reader
/// [`crate::openhuman::tinyagents::journal::read_run_events`] uses). Best-effort:
/// a missing store or unknown run yields an empty page, not an error.
pub(crate) async fn read_run_events_page(
    workspace: &Path,
    run_id: &str,
    offset: u64,
    limit: u64,
) -> anyhow::Result<RunEventsPage> {
    // Guard the page size: clamp a zero/absurd limit into [1, MAX].
    let effective_limit = limit.clamp(1, MAX_EVENTS_LIMIT);
    log::debug!(
        "[agent] replay read_run_events_page run_id={run_id} offset={offset} \
         limit={limit} effective_limit={effective_limit}"
    );

    let stores = open_session_stores(workspace);
    let journal = StoreEventJournal::new(stores.journal);
    // Read one extra record to detect whether a further page exists without a
    // second store round-trip.
    let mut events = journal.read_from(run_id, offset).await.map_err(|e| {
        anyhow::anyhow!("[agent] replay read_run_events_page failed run_id={run_id}: {e}")
    })?;

    let has_more = events.len() as u64 > effective_limit;
    if has_more {
        events.truncate(effective_limit as usize);
    }
    // Offsets are monotonic within a run, so the cursor is simply "one past the
    // last returned offset". `None` when this page drained the stream.
    let next_offset = if has_more {
        events.last().map(|obs| obs.offset + 1)
    } else {
        None
    };

    log::debug!(
        "[agent] replay read_run_events_page run_id={run_id} returned={} next_offset={:?}",
        events.len(),
        next_offset
    );
    Ok(RunEventsPage {
        events,
        next_offset,
    })
}

/// Latest durable [`HarnessRunStatus`] for `run_id`, or `None` when the run is
/// unknown. Backed by the C4 status seam
/// ([`crate::openhuman::tinyagents::journal::read_run_status`] /
/// [`FileStatusStore::get_status`]).
pub(crate) async fn read_run_status(
    workspace: &Path,
    run_id: &str,
) -> anyhow::Result<Option<HarnessRunStatus>> {
    log::debug!("[agent] replay read_run_status run_id={run_id}");
    let stores = open_session_stores(workspace);
    let store = FileStatusStore::new(stores.kv);
    let status = store.get_status(run_id).await.map_err(|e| {
        anyhow::anyhow!("[agent] replay read_run_status failed run_id={run_id}: {e}")
    })?;
    log::debug!(
        "[agent] replay read_run_status run_id={run_id} found={}",
        status.is_some()
    );
    Ok(status)
}

/// Is a run still live (i.e. eligible for the "active" listing)?
///
/// Mirrors the liveness predicate the crate's status store uses for
/// `list_active` (Pending / Running / Interrupted).
fn is_active(status: &HarnessRunStatus) -> bool {
    matches!(
        status.status,
        ExecutionStatus::Pending | ExecutionStatus::Running | ExecutionStatus::Interrupted
    )
}

/// Active runs, optionally filtered by `thread_id` and/or `root_run_id`.
///
/// Backed by the C4 status seam:
/// - no filter → [`FileStatusStore::list_active`]
/// - `thread_id` → [`FileStatusStore::list_by_thread`]
/// - `root_run_id` → [`FileStatusStore::list_by_root`]
///
/// The thread/root store queries return *all* runs (active and terminal), so the
/// active-liveness predicate is always applied on top — this controller only
/// ever surfaces live runs. When both filters are supplied, the base query uses
/// `thread_id` and the result is further restricted to `root_run_id`.
pub(crate) async fn list_active_runs(
    workspace: &Path,
    thread_id: Option<&str>,
    root_run_id: Option<&str>,
) -> anyhow::Result<Vec<HarnessRunStatus>> {
    log::debug!(
        "[agent] replay list_active_runs thread_id={:?} root_run_id={:?}",
        thread_id,
        root_run_id
    );
    let stores = open_session_stores(workspace);
    let store = FileStatusStore::new(stores.kv);

    let base = match (thread_id, root_run_id) {
        (Some(thread), _) => store.list_by_thread(thread).await,
        (None, Some(root)) => store.list_by_root(root).await,
        (None, None) => store.list_active().await,
    }
    .map_err(|e| anyhow::anyhow!("[agent] replay list_active_runs failed: {e}"))?;

    let mut runs: Vec<HarnessRunStatus> = base.into_iter().filter(is_active).collect();
    // If a caller supplied BOTH a thread and a root, the thread query drove the
    // base list; narrow it to the requested root as well.
    if thread_id.is_some() {
        if let Some(root) = root_run_id {
            runs.retain(|s| s.root_run_id.as_str() == root);
        }
    }

    log::debug!("[agent] replay list_active_runs returned={}", runs.len());
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use tinyagents::harness::events::{AgentEvent, EventSink};
    use tinyagents::harness::ids::{ComponentId, HarnessPhase, ThreadId};
    use tinyagents::harness::observability::{FanOutSink, JournalSink, StoreEventJournal};

    use crate::openhuman::tinyagents::journal::mint_run_id;

    /// Seed `count` durable events for a fresh run under `workspace`, returning
    /// the run id. Mirrors the seam wiring in the `journal.rs` tests: a run
    /// [`EventSink`] seeded with the run id (so persisted `event_id`s are the
    /// stable `{run_id}-evt-{offset}`) fanning out into a [`StoreEventJournal`].
    async fn seed_run_events(workspace: &Path, count: usize) -> String {
        let stores = open_session_stores(workspace);
        let run_id = mint_run_id();
        let journal: Arc<dyn HarnessEventJournal> =
            Arc::new(StoreEventJournal::new(stores.journal));
        let sink = EventSink::with_stream_id(run_id.as_str());
        let journal_sink = JournalSink::new(journal, run_id.clone());
        sink.subscribe(Arc::new(FanOutSink::new().with(Arc::new(journal_sink))));
        for i in 0..count {
            sink.emit(AgentEvent::ToolStarted {
                call_id: format!("c{i}").into(),
                tool_name: format!("tool-{i}"),
            });
        }
        run_id.as_str().to_string()
    }

    /// Paging boundary: a page smaller than the stream reports a `next_offset`
    /// cursor; the final page drains to `None` and never over-reads.
    #[tokio::test]
    async fn read_run_events_page_pages_and_drains() {
        let tmp = std::env::temp_dir().join(format!("oh-replay-page-{}", uuid::Uuid::new_v4()));
        let run_id = seed_run_events(&tmp, 3).await;

        // First page (limit 2) returns offsets 0,1 with a cursor at offset 2.
        let page1 = read_run_events_page(&tmp, &run_id, 0, 2).await.unwrap();
        assert_eq!(page1.events.len(), 2);
        assert_eq!(page1.events[0].offset, 0);
        assert_eq!(page1.events[1].offset, 1);
        assert_eq!(page1.next_offset, Some(2), "more events remain");

        // Second page resumes at the cursor and drains → next_offset None.
        let page2 = read_run_events_page(&tmp, &run_id, page1.next_offset.unwrap(), 2)
            .await
            .unwrap();
        assert_eq!(page2.events.len(), 1);
        assert_eq!(page2.events[0].offset, 2);
        assert_eq!(page2.next_offset, None, "stream drained on the last page");

        // A page exactly the size of the remaining stream still drains to None
        // (no phantom extra page).
        let exact = read_run_events_page(&tmp, &run_id, 0, 3).await.unwrap();
        assert_eq!(exact.events.len(), 3);
        assert_eq!(exact.next_offset, None);

        // Unknown run → empty page, not an error.
        let empty = read_run_events_page(&tmp, "run.does-not-exist", 0, 10)
            .await
            .unwrap();
        assert!(empty.events.is_empty());
        assert_eq!(empty.next_offset, None);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Status reader returns `None` for a run that was never recorded.
    #[tokio::test]
    async fn read_run_status_none_for_unknown_run() {
        let tmp = std::env::temp_dir().join(format!("oh-replay-status-{}", uuid::Uuid::new_v4()));
        let missing = read_run_status(&tmp, "run.nope").await.unwrap();
        assert!(missing.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Active listing surfaces a started run and filters by thread; a completed
    /// run is excluded.
    #[tokio::test]
    async fn list_active_runs_returns_started_and_filters_by_thread() {
        let tmp = std::env::temp_dir().join(format!("oh-replay-active-{}", uuid::Uuid::new_v4()));
        let store = FileStatusStore::new(open_session_stores(&tmp).kv);

        // A running run on thread-A.
        let run_a = mint_run_id();
        let mut status_a =
            HarnessRunStatus::new(run_a.clone(), ComponentId::new("mock-model".to_string()))
                .with_thread(ThreadId::new("thread-A"));
        status_a.mark_running(HarnessPhase::Model);
        store.put_status(status_a).await.unwrap();

        // A completed run on thread-B (must NOT appear in the active listing).
        let run_b = mint_run_id();
        let mut status_b =
            HarnessRunStatus::new(run_b.clone(), ComponentId::new("mock-model".to_string()))
                .with_thread(ThreadId::new("thread-B"));
        status_b.mark_running(HarnessPhase::Model);
        status_b.mark_completed();
        store.put_status(status_b).await.unwrap();

        // No filter: only the running run.
        let active = list_active_runs(&tmp, None, None).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id.as_str(), run_a.as_str());

        // Filter by thread-A: the running run is returned.
        let by_thread_a = list_active_runs(&tmp, Some("thread-A"), None)
            .await
            .unwrap();
        assert_eq!(by_thread_a.len(), 1);
        assert_eq!(by_thread_a[0].run_id.as_str(), run_a.as_str());

        // Filter by thread-B: the only run there is completed → excluded.
        let by_thread_b = list_active_runs(&tmp, Some("thread-B"), None)
            .await
            .unwrap();
        assert!(by_thread_b.is_empty());

        // Filter by an unknown thread: empty.
        let by_thread_none = list_active_runs(&tmp, Some("nope"), None).await.unwrap();
        assert!(by_thread_none.is_empty());

        // Filter by root_run_id (a top-level run's root equals its own id).
        let by_root = list_active_runs(&tmp, None, Some(run_a.as_str()))
            .await
            .unwrap();
        assert_eq!(by_root.len(), 1);
        assert_eq!(by_root[0].run_id.as_str(), run_a.as_str());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
