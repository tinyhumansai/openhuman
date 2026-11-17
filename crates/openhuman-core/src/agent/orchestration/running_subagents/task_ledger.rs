//! Durable per-workspace task store plumbing and the typed lifecycle ledger
//! that mirrors each detached sub-agent's status into it (issue #4249).
//!
//! The store is TinyAgents' `DriverTaskStore` over the storage backend's
//! documents, cached per workspace and scope (`TaskStoreRegistry`); a backend
//! that cannot be reached degrades to an in-memory ledger rather than taking
//! orchestration down. What stays here is the record shape and the
//! reconciliation of orphaned tasks on boot.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use tinyagents_orchestration::subagent::{
    orphaned_subagent_reason, record_agent_id, record_parent_session, task_status_label, WaitError,
};
use tinyagents_tasks::{
    open_driver_task_store_or_memory, reconcile_orphaned_tasks, InMemoryTaskStore,
    OrchestrationTaskFilter, TaskStore, TaskStoreRegistry,
};

/// One cached ledger: a workspace and the storage scope it was opened for.
type LedgerKey = (PathBuf, String);

const DOMAIN: &str = "running_subagents::task_ledger";

/// Process-wide typed lifecycle ledger for detached sub-agents (issue #4249),
/// one durable store per workspace and storage scope: the `orchestration_*`
/// document collections of the storage backend (TinyAgents' `DriverTaskStore`).
static TASK_STORES: OnceLock<TaskStoreRegistry<LedgerKey>> = OnceLock::new();

fn task_stores() -> &'static TaskStoreRegistry<LedgerKey> {
    TASK_STORES.get_or_init(|| {
        TaskStoreRegistry::new(|(workspace_dir, scope): &LedgerKey| {
            match open_driver_store(workspace_dir) {
                Ok(store) => store,
                Err(error) => {
                    // A backend that cannot be reached degrades to an
                    // in-memory ledger rather than taking orchestration down.
                    log::warn!(
                        "[running_subagents] task ledger unavailable for scope={scope}; using memory: {error:#}"
                    );
                    Arc::new(InMemoryTaskStore::new())
                }
            }
        })
    })
}

fn open_driver_store(workspace_dir: &Path) -> anyhow::Result<Arc<dyn TaskStore>> {
    let opened = crate::storage::local::open(DOMAIN, workspace_dir)?;
    let scoped = opened.scoped()?;
    Ok(open_driver_task_store_or_memory(Arc::clone(
        scoped.documents(),
    )))
}

/// The cache key for this call: the storage scope joins the workspace, so two
/// agents on one workspace never share a ledger.
fn ledger_key(workspace_dir: &Path) -> Option<LedgerKey> {
    match crate::storage::current_scope() {
        Ok(scope) => Some((workspace_dir.to_path_buf(), scope.to_string())),
        Err(error) => {
            log::warn!("[running_subagents] task ledger has no storage scope: {error}");
            None
        }
    }
}

/// The ledger for `workspace_dir`, opening it on first use.
///
/// A poisoned registry lock is degraded to a throwaway in-memory store rather
/// than propagated: every caller here is a best-effort bookkeeping path, and a
/// panic in an unrelated task must not turn sub-agent spawning into a second
/// panic.
pub(crate) fn task_store_for_workspace(workspace_dir: &Path) -> Arc<dyn TaskStore> {
    let Some(key) = ledger_key(workspace_dir) else {
        return Arc::new(InMemoryTaskStore::new());
    };
    match task_stores().get_or_open(&key) {
        Ok(store) => store,
        Err(err) => {
            log::warn!(
                "[running_subagents] task store registry unavailable for {}; using a detached in-memory ledger: {}",
                workspace_dir.display(),
                err
            );
            Arc::new(InMemoryTaskStore::new())
        }
    }
}

/// Open the caller's task store in its captured scope. SaaS calls without a
/// resolved owner fail closed rather than falling back to ambient storage.
pub(super) fn scoped_task_store(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &Path,
) -> Result<Arc<dyn TaskStore>, WaitError> {
    if let Some(context) = ctx.resolved() {
        return Ok(crate::core::runtime::CoreContext::sync_scope(
            context,
            || task_store_for_workspace(workspace_dir),
        ));
    }
    if crate::core::runtime::is_saas() {
        return Err(WaitError::NotOwned);
    }
    Ok(task_store_for_workspace(workspace_dir))
}

/// Restart/resume reconciliation for detached sub-agents (issue #4249 / 07.2
/// steps 2 & 4).
///
/// A detached sub-agent runs as a `tokio` task owned by the process that spawned
/// it. When the core restarts, that task — and its live [`tokio::task::AbortHandle`] /
/// [`tinyagents_harness::CancellationToken`] — is gone, but the durable
/// task ledger still holds a non-terminal (`Pending`/`Running`/`Awaiting`/
/// `CancelRequested`) record for it. Such a record is **orphaned**: there is no
/// live executor to re-attach to (OpenHuman spawns child processes, so an
/// in-flight run from a dead parent cannot be resumed), and the run-ledger
/// finalizer never observed a terminal event, so it would otherwise render as a
/// perpetual "running" entry.
///
/// This scans the workspace-scoped store for those orphans and reconciles each
/// to a terminal state — `Cancelled` if a cancel had been requested, otherwise
/// `Failed` with an "orphaned by restart" reason — then emits the existing typed
/// terminal lifecycle event ([`crate::agent::orchestration::subagent_events::publish_subagent_failed`])
/// so the run ledger finalizes. Best-effort and non-fatal: per-task transition
/// errors (e.g. a record that raced to terminal) are logged and skipped, and a
/// store-open failure simply reconciles nothing. Returns the count reconciled.
pub(crate) fn reconcile_orphaned_tasks_on_boot(workspace_dir: &Path) -> usize {
    // On a backend several cores share, a non-terminal task in the ledger may
    // belong to another live process, not to a dead one of ours; settling it
    // would fail work that is still running. Only a backend this process owns
    // (files, SQLite, memory) can be swept.
    if crate::storage::installed_is_shared() {
        log::debug!(
            "[running_subagents] skipping orphan reconcile: storage backend is shared workspace_dir={}",
            workspace_dir.display()
        );
        return 0;
    }
    let store = task_store_for_workspace(workspace_dir);

    // The sweep itself — which statuses are live, and which terminal state each
    // becomes — is TinyAgents'. What stays here is the reason a *sub-agent*
    // orphan carries, and the lifecycle event that finalizes OpenHuman's run
    // ledger afterwards.
    let report = reconcile_orphaned_tasks(
        store.as_ref(),
        OrchestrationTaskFilter::default().with_kind("sub_agent"),
        &|record| orphaned_subagent_reason(record.status),
    );

    if report.is_empty() {
        log::debug!(
            "[running_subagents] reconcile found no orphaned sub-agent tasks workspace_dir={}",
            workspace_dir.display()
        );
        return 0;
    }

    for task in report.settled() {
        let task_id = task.task_id.as_str().to_string();
        let prior = task_status_label(task.prior_status);
        let reason = orphaned_subagent_reason(task.prior_status);
        let parent_session = record_parent_session(&task.record)
            .unwrap_or_default()
            .to_string();
        let agent_id = record_agent_id(&task.record);
        // Reuse the 05.2 typed terminal lifecycle helper so the run ledger
        // finalizes exactly as it would for a live failure.
        crate::agent::orchestration::subagent_events::publish_subagent_failed(
            parent_session,
            task_id.clone(),
            agent_id,
            reason,
        );
        log::info!(
            "[running_subagents] reconciled orphaned sub-agent task_id={} prior_status={} -> terminal",
            task_id,
            prior
        );
    }

    let reconciled = report.reconciled_count();
    log::info!(
        "[running_subagents] reconciled {reconciled} orphaned sub-agent task(s) on boot workspace_dir={} errors={}",
        workspace_dir.display(),
        report.error_count()
    );
    reconciled
}
