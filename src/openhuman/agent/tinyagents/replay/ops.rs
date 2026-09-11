//! Read-only business logic for the agent replay/status RPC surface
//! (workstream 05.x). Every function here is a *reader* over the C4 durable
//! journal + status seams built in
//! [`crate::openhuman::agent::tinyagents::journal`] — it opens the same
//! `{workspace}/tinyagents_store/{kv,journal}` stores and never writes, mutates,
//! or bypasses any security/approval/sandbox gate.
//!
//! The controller layer ([`super::schemas`]) resolves the configured workspace
//! and delegates here; these functions take an explicit `workspace` path so they
//! are unit-testable against a temp store (mirroring the `journal.rs` tests).

use std::path::Path;

use tinyagents_harness::events::HarnessRunStatus;
use tinyagents_harness::ids::ExecutionStatus;
use tinyagents_harness::observability::{
    AgentObservation, HarnessEventJournal, HarnessStatusStore, StoreEventJournal,
};

use crate::openhuman::agent::session_import::ops::open_session_stores;
use crate::openhuman::agent::tinyagents::journal::FileStatusStore;

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
/// [`crate::openhuman::agent::tinyagents::journal::read_run_events`] uses). Best-effort:
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
/// ([`crate::openhuman::agent::tinyagents::journal::read_run_status`] /
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
#[path = "ops_tests.rs"]
mod tests;
