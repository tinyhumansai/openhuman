//! Startup reconciliation for orphaned agent runs.
//!
//! The sweep itself (`tinyagents_harness::observability::reap_orphaned_runs`)
//! lives upstream; this host adapter only opens the tenant's durable status
//! store. It is the one *writer* over the status seam that
//! [`crate::agent::tinyagents::journal`] exposes; the replay/status controllers
//! ([`super::replay`]) stay strictly read-only.
//!
//! The boot path starts the sweep with [`spawn_startup_sweep`] and does not
//! wait for it: with a storage backend it visits every recorded agent one
//! after another, and on a host with hundreds of agents that took minutes.
//! Because the sweep now overlaps this process's own work, it only reaps runs
//! that started before the sweep was spawned ([`StartedBefore`]); a run this
//! process starts meanwhile is never touched.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use tinyagents_harness::events::HarnessRunStatus;
use tinyagents_harness::observability::HarnessStatusStore;

/// Start the orphaned-run sweep in the background and return at once.
///
/// Called at the end of `CoreBuilder::build` (and after the secret
/// preflight), so build-only embedders still get the sweep, but boot no
/// longer waits on it. Runs on the current tokio runtime; without one (a
/// caller driving `build()` on another executor) it returns `None` and the
/// caller should run [`reap_orphaned_runs`] inline.
pub(crate) fn spawn_startup_sweep(workspace: PathBuf) -> Option<tokio::task::JoinHandle<usize>> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    // Fixed before the task starts: everything that started earlier belongs
    // to a previous process; everything later may be this process's own.
    let cutoff = SystemTime::now();
    Some(handle.spawn(async move {
        #[cfg(test)]
        wait_for_sweep_gate().await;
        reap_orphaned_runs_before(&workspace, Some(cutoff)).await
    }))
}

/// Reap every run left non-terminal by a previous process, returning the number
/// of runs moved to `Cancelled`. Best-effort; never fails.
///
/// Skipped on a storage backend other processes may share (MongoDB): there a
/// non-terminal status can belong to a run another replica is still driving,
/// and cancelling it would hide that run from active and late-attach status.
///
/// With a storage backend every agent keeps its own status store, so the
/// sweep visits `local` and then each known agent (`crate::storage::agents`).
pub(crate) async fn reap_orphaned_runs(workspace: &Path) -> usize {
    reap_orphaned_runs_before(workspace, None).await
}

/// What a sweep did: how many storage scopes it visited and how many runs it
/// moved to `Cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SweepOutcome {
    scopes: usize,
    reaped: usize,
}

/// [`reap_orphaned_runs`], limited to runs that started before `cutoff` when
/// one is given (the spawned sweep, which overlaps this process's own work).
async fn reap_orphaned_runs_before(workspace: &Path, cutoff: Option<SystemTime>) -> usize {
    sweep(workspace, cutoff).await.reaped
}

async fn sweep(workspace: &Path, cutoff: Option<SystemTime>) -> SweepOutcome {
    let started = Instant::now();
    let shared = crate::storage::installed_is_shared();
    log::info!(
        "[agent] startup run sweep start workspace={} shared={shared}",
        workspace.display()
    );
    if shared {
        // Visiting a scope prepares its collections and migrations, and there
        // is nothing to reap on a backend other processes drive: skip them
        // all instead of visiting every agent to do nothing.
        log::info!("[agent] startup run sweep skipped: the storage backend is shared");
        return SweepOutcome {
            scopes: 0,
            reaped: 0,
        };
    }
    let results = crate::storage::agents::for_each_scope(
        &crate::core::runtime::DispatchCtx::host_default(),
        "run reaper",
        |_| reap_unless_shared(workspace, shared, cutoff),
    )
    .await;
    let scopes = results.len();
    let reaped: usize = results.into_iter().map(|(_, reaped)| reaped).sum();
    log::info!(
        "[agent] startup run sweep done scopes={scopes} reaped={reaped} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    SweepOutcome { scopes, reaped }
}

async fn reap_unless_shared(workspace: &Path, shared: bool, cutoff: Option<SystemTime>) -> usize {
    if shared {
        log::info!("[agent] startup run sweep skipped: the storage backend is shared");
        return 0;
    }
    log::debug!(
        "[agent] startup run sweep workspace={}",
        workspace.display()
    );
    // The status records live in the storage backend, in the calling
    // tenant's scope: sweep the ones this process's own work wrote there.
    let Some(store) = crate::storage::sessions::status_store_for(workspace) else {
        log::warn!("[agent] startup run sweep skipped: no authorized status store");
        return 0;
    };
    match cutoff {
        Some(cutoff) => {
            tinyagents_harness::observability::reap_orphaned_runs(&StartedBefore::new(
                store, cutoff,
            ))
            .await
        }
        None => tinyagents_harness::observability::reap_orphaned_runs(&store).await,
    }
}

/// A status store whose active listing only shows runs that started before
/// `cutoff`, so the upstream sweep cannot cancel a run this process started
/// while the sweep was still in flight. Everything else passes through.
///
/// `started_at` is persisted in whole seconds, so the cutoff is rounded down
/// to a whole second: a run started in the same second as the sweep is left
/// alone, whichever process started it.
struct StartedBefore<S> {
    inner: S,
    cutoff: SystemTime,
}

impl<S> StartedBefore<S> {
    fn new(inner: S, cutoff: SystemTime) -> Self {
        let whole_seconds = cutoff
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| SystemTime::UNIX_EPOCH + Duration::from_secs(d.as_secs()))
            .unwrap_or(cutoff);
        Self {
            inner,
            cutoff: whole_seconds,
        }
    }
}

#[async_trait]
impl<S: HarnessStatusStore> HarnessStatusStore for StartedBefore<S> {
    async fn put_status(&self, status: HarnessRunStatus) -> tinyagents_harness::Result<()> {
        self.inner.put_status(status).await
    }

    async fn get_status(
        &self,
        run_id: &str,
    ) -> tinyagents_harness::Result<Option<HarnessRunStatus>> {
        self.inner.get_status(run_id).await
    }

    async fn list_by_thread(
        &self,
        thread_id: &str,
    ) -> tinyagents_harness::Result<Vec<HarnessRunStatus>> {
        self.inner.list_by_thread(thread_id).await
    }

    async fn list_by_root(
        &self,
        root_run_id: &str,
    ) -> tinyagents_harness::Result<Vec<HarnessRunStatus>> {
        self.inner.list_by_root(root_run_id).await
    }

    async fn list_active(&self) -> tinyagents_harness::Result<Vec<HarnessRunStatus>> {
        let mut active = self.inner.list_active().await?;
        active.retain(|status| status.started_at < self.cutoff);
        Ok(active)
    }
}

/// Lets a test hold the spawned sweep before it touches the store, to prove
/// boot does not wait for it. Closed by [`hold_startup_sweeps_for_test`].
#[cfg(test)]
static SWEEP_GATE: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();

/// Hold every sweep spawned from now on until [`release_startup_sweeps_for_test`].
#[cfg(test)]
pub(crate) fn hold_startup_sweeps_for_test() {
    let _ = SWEEP_GATE.set(tokio::sync::Semaphore::new(0));
}

/// Let held sweeps run.
#[cfg(test)]
pub(crate) fn release_startup_sweeps_for_test() {
    if let Some(gate) = SWEEP_GATE.get() {
        gate.add_permits(tokio::sync::Semaphore::MAX_PERMITS / 2);
    }
}

#[cfg(test)]
async fn wait_for_sweep_gate() {
    if let Some(gate) = SWEEP_GATE.get() {
        let _ = gate.acquire().await;
    }
}

#[cfg(test)]
#[path = "reaper_tests.rs"]
mod tests;
