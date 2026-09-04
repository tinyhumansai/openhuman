//! Startup reconciliation for orphaned agent runs.
//!
//! The core is a single in-process runtime. When it exits — a crash, a restart,
//! or a deploy — any run still in a non-terminal state (`Pending` / `Running` /
//! `Interrupted`) in the durable status store is *orphaned*: no executor will
//! ever advance it, yet it lingers in the "active runs" listing
//! (`openhuman.agent_runs_active`) indefinitely. A long-lived instance was
//! observed carrying 50 such zombies, all invisible to any cancel path because
//! the task that owned each run no longer exists.
//!
//! On startup we sweep every still-active run to the terminal `Cancelled` state
//! with an explanatory error so the active listing reflects reality. This is
//! the one *writer* over the status seam that
//! [`crate::openhuman::agent::tinyagents::journal`] exposes; the replay/status
//! controllers ([`super::replay`]) stay strictly read-only.

use std::path::Path;
use std::time::SystemTime;

use tinyagents_harness::events::HarnessRunStatus;
use tinyagents_harness::ids::{ExecutionStatus, HarnessPhase};
use tinyagents_harness::observability::HarnessStatusStore;

use crate::openhuman::agent::session_import::ops::open_session_stores;
use crate::openhuman::agent::tinyagents::journal::FileStatusStore;

/// Error recorded on a run reaped by the startup sweep. Stable + grep-friendly
/// so an operator (or a test) can tell a reaped run from a genuinely failed one.
pub(crate) const ORPHAN_REAP_REASON: &str =
    "run orphaned: core restarted while the run was in flight";

/// Reap every run left non-terminal by a previous process.
///
/// Opens the durable status store under `workspace`, lists the still-active
/// runs, and moves each to the terminal `Cancelled` state. Best-effort: a
/// per-run persistence failure is logged and does not abort the sweep, and a
/// failure to open/list the store logs and yields `0` rather than blocking
/// boot. Returns the number of runs reaped.
pub(crate) async fn reap_orphaned_runs(workspace: &Path) -> usize {
    // One id for the whole sweep so every line below can be tied to the same
    // boot — a restart loop otherwise interleaves two sweeps' lines with nothing
    // to tell them apart. Derived from the clock rather than a uuid so it stays
    // ordered in a log tail.
    let sweep_id = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    log::debug!(
        "[agent] startup run sweep entry sweep_id={sweep_id} workspace={}",
        workspace.display()
    );

    let stores = open_session_stores(workspace);
    let store = FileStatusStore::new(stores.kv);

    let active = match store.list_active().await {
        Ok(runs) => runs,
        Err(err) => {
            log::warn!(
                "[agent] startup run sweep exit sweep_id={sweep_id} branch=list-failed: {err}"
            );
            return 0;
        }
    };

    if active.is_empty() {
        log::debug!("[agent] startup run sweep exit sweep_id={sweep_id} branch=none-active");
        return 0;
    }
    log::debug!(
        "[agent] startup run sweep sweep_id={sweep_id} active={} to reap",
        active.len()
    );

    let mut reaped = 0usize;
    for mut status in active {
        let run_id = status.run_id.as_str().to_string();
        // The source state is read before the mutation, not assumed: `list_active`
        // returns whatever non-terminal state a run was left in, and logging a
        // presumed `Running` would misreport a run that died mid-`Queued`.
        let from_status = format!("{:?}", status.status);
        mark_orphaned(&mut status);
        let to_status = format!("{:?}", status.status);
        log::debug!(
            "[agent] startup run sweep sweep_id={sweep_id} run_id={run_id} \
             transition={from_status}->{to_status} phase=Done persisting"
        );
        match store.put_status(status).await {
            Ok(()) => {
                reaped += 1;
                log::debug!(
                    "[agent] startup run sweep sweep_id={sweep_id} run_id={run_id} \
                     transition={from_status}->{to_status} persisted"
                );
            }
            Err(err) => {
                log::warn!(
                    "[agent] startup run sweep sweep_id={sweep_id} run_id={run_id} \
                     transition={from_status}->{to_status} failed: {err}"
                );
            }
        }
    }
    // One operator-visible line for the whole sweep, and only when it did
    // something: a clean boot is the normal case and says nothing.
    if reaped > 0 {
        log::info!("[agent] startup run sweep sweep_id={sweep_id} reaped {reaped} orphaned run(s)");
    } else {
        log::debug!("[agent] startup run sweep exit sweep_id={sweep_id} branch=nothing-reaped");
    }
    reaped
}

/// Move a run to the terminal `Cancelled` state with the orphan reason.
///
/// tinyagents exposes `mark_completed` / `mark_failed` / `mark_interrupted` but
/// no `mark_cancelled`; the `HarnessRunStatus` fields are public, so we set the
/// terminal state directly here rather than take a cross-repo dependency on a
/// new helper. `Cancelled` (not `Failed`) is deliberate: a restart is not a run
/// failure, and marking it `Failed` would render every reaped run as a red
/// error row in the UI. Mirrors the field writes `mark_failed` performs
/// (terminal status + `Done` phase + `error` + `ended_at`/`updated_at`).
fn mark_orphaned(status: &mut HarnessRunStatus) {
    status.status = ExecutionStatus::Cancelled;
    status.current_phase = HarnessPhase::Done;
    status.error = Some(ORPHAN_REAP_REASON.to_string());
    let now = SystemTime::now();
    status.ended_at = Some(now);
    status.updated_at = now;
}

#[cfg(test)]
#[path = "reaper_tests.rs"]
mod tests;
