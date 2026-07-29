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
//! [`crate::openhuman::tinyagents::journal`] exposes; the replay/status
//! controllers ([`super::replay`]) stay strictly read-only.

use std::path::Path;
use std::time::SystemTime;

use tinyagents::harness::events::HarnessRunStatus;
use tinyagents::harness::ids::{ExecutionStatus, HarnessPhase};
use tinyagents::harness::observability::HarnessStatusStore;

use crate::openhuman::session_import::ops::open_session_stores;
use crate::openhuman::tinyagents::journal::FileStatusStore;

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
    let stores = open_session_stores(workspace);
    let store = FileStatusStore::new(stores.kv);

    let active = match store.list_active().await {
        Ok(runs) => runs,
        Err(err) => {
            log::warn!("[agent] startup run sweep: list_active failed: {err}");
            return 0;
        }
    };

    if active.is_empty() {
        log::debug!("[agent] startup run sweep: no orphaned runs");
        return 0;
    }
    log::info!(
        "[agent] startup run sweep: {} orphaned run(s) to reap",
        active.len()
    );

    let mut reaped = 0usize;
    for mut status in active {
        let run_id = status.run_id.as_str().to_string();
        mark_orphaned(&mut status);
        match store.put_status(status).await {
            Ok(()) => {
                reaped += 1;
                log::debug!("[agent] startup run sweep: reaped run_id={run_id}");
            }
            Err(err) => {
                log::warn!("[agent] startup run sweep: reap failed run_id={run_id}: {err}");
            }
        }
    }
    log::info!("[agent] startup run sweep: reaped {reaped} orphaned run(s)");
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
mod tests {
    use super::*;

    use tinyagents::harness::ids::ComponentId;

    use crate::openhuman::tinyagents::journal::mint_run_id;

    /// Build a fresh status in the given non-terminal state and persist it.
    async fn seed_status(store: &FileStatusStore, status_kind: ExecutionStatus) -> String {
        let run_id = mint_run_id();
        let mut status =
            HarnessRunStatus::new(run_id.clone(), ComponentId::new("mock-model".to_string()));
        match status_kind {
            ExecutionStatus::Pending => { /* fresh status is already Pending */ }
            ExecutionStatus::Running => status.mark_running(HarnessPhase::Model),
            ExecutionStatus::Interrupted => status.mark_interrupted(),
            other => panic!("seed_status only seeds non-terminal states, got {other:?}"),
        }
        store.put_status(status).await.unwrap();
        run_id.as_str().to_string()
    }

    /// The sweep reaps every non-terminal run to `Cancelled` with the reason,
    /// leaves terminal runs untouched, and empties the active listing.
    #[tokio::test]
    async fn reap_cancels_every_active_run_and_spares_terminal_ones() {
        let tmp = std::env::temp_dir().join(format!("oh-reaper-{}", uuid::Uuid::new_v4()));
        let store = FileStatusStore::new(open_session_stores(&tmp).kv);

        let pending = seed_status(&store, ExecutionStatus::Pending).await;
        let running = seed_status(&store, ExecutionStatus::Running).await;
        let interrupted = seed_status(&store, ExecutionStatus::Interrupted).await;

        // A run that already finished must survive the sweep unchanged.
        let done = mint_run_id();
        let mut done_status =
            HarnessRunStatus::new(done.clone(), ComponentId::new("mock-model".to_string()));
        done_status.mark_running(HarnessPhase::Model);
        done_status.mark_completed();
        store.put_status(done_status).await.unwrap();

        let reaped = reap_orphaned_runs(&tmp).await;
        assert_eq!(reaped, 3, "the three non-terminal runs were reaped");

        // Every orphan is now terminal-cancelled with the stable reason.
        for run_id in [&pending, &running, &interrupted] {
            let status = store
                .get_status(run_id)
                .await
                .unwrap()
                .expect("status present");
            assert_eq!(status.status, ExecutionStatus::Cancelled);
            assert_eq!(status.current_phase, HarnessPhase::Done);
            assert_eq!(status.error.as_deref(), Some(ORPHAN_REAP_REASON));
            assert!(status.ended_at.is_some(), "reaped run has an end time");
        }

        // The completed run is left exactly as it was.
        let done_after = store
            .get_status(done.as_str())
            .await
            .unwrap()
            .expect("done present");
        assert_eq!(done_after.status, ExecutionStatus::Completed);
        assert!(done_after.error.is_none());

        // The active listing is now empty — a second sweep is a no-op.
        assert!(store.list_active().await.unwrap().is_empty());
        assert_eq!(
            reap_orphaned_runs(&tmp).await,
            0,
            "idempotent: nothing left to reap"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A workspace that never hosted a run reaps nothing and does not error.
    #[tokio::test]
    async fn reap_on_empty_workspace_is_a_noop() {
        let tmp = std::env::temp_dir().join(format!("oh-reaper-empty-{}", uuid::Uuid::new_v4()));
        assert_eq!(reap_orphaned_runs(&tmp).await, 0);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
