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
    list_subagent_records, orphaned_subagent_reason, record_agent_id,
    record_cancelled as ledger_cancelled, record_parent_session, record_spawned as ledger_spawned,
    subagent_record_for_task, task_status_label, SpawnedSubagent, WaitError,
};
use tinyagents_tasks::{
    reconcile_orphaned_tasks, DriverTaskStore, InMemoryTaskStore, OrchestrationTaskFilter,
    OrchestrationTaskRecord, TaskStore, TaskStoreRegistry,
};

#[cfg(test)]
fn default_task_store_workspace() -> PathBuf {
    crate::config::default_root_openhuman_dir()
        .map(|root| root.join("workspace"))
        .unwrap_or_else(|_| PathBuf::from(".openhuman").join("workspace"))
}

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
    let store = DriverTaskStore::new(Arc::clone(scoped.documents()))
        .map_err(|error| anyhow::anyhow!("start the task store bridge: {error}"))?;
    Ok(Arc::new(store))
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

#[cfg(test)]
fn task_store() -> Arc<dyn TaskStore> {
    let workspace = default_task_store_workspace();
    task_store_for_workspace(&workspace)
}

/// Record a freshly-spawned sub-agent in the workspace's store (`Pending` →
/// `Running`). Insert errors (e.g. a re-used task id across tests) are ignored.
pub(crate) fn record_spawned(
    task_id: &str,
    agent_id: &str,
    parent_session: &str,
    session_parent_prefix: Option<&str>,
    subagent_session_id: Option<&str>,
    workspace_dir: &Path,
    parent_thread_id: Option<&str>,
) {
    let workspace = workspace_dir.display().to_string();
    if let Err(err) = ledger_spawned(
        task_store_for_workspace(workspace_dir).as_ref(),
        &SpawnedSubagent {
            task_id,
            agent_id,
            parent_session,
            session_parent_prefix,
            subagent_session_id,
            workspace_dir: &workspace,
            parent_thread_id,
        },
    ) {
        log::debug!(
            "[running_subagents] spawn ledger insert ignored task_id={task_id} error={err}"
        );
    }
}

/// Record a cancellation (`CancelRequested` → `Cancelled`) for `task_id`.
pub(crate) fn record_cancelled(workspace_dir: &Path, task_id: &str) {
    log::debug!(
        "[running_subagents] recording task cancellation task_id={} workspace_dir={}",
        task_id,
        workspace_dir.display()
    );
    if let Err(err) = ledger_cancelled(task_store_for_workspace(workspace_dir).as_ref(), task_id) {
        log::debug!(
            "[running_subagents] cancel ledger update ignored task_id={task_id} error={err}"
        );
    }
}

pub(crate) fn list_task_records(workspace_dir: &Path) -> Vec<OrchestrationTaskRecord> {
    list_subagent_records(task_store_for_workspace(workspace_dir).as_ref())
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

pub(crate) fn task_record_for_task_in_workspace(
    workspace_dir: &Path,
    task_id: &str,
    parent_session: &str,
) -> Result<OrchestrationTaskRecord, WaitError> {
    subagent_record_for_task(
        task_store_for_workspace(workspace_dir).as_ref(),
        task_id,
        parent_session,
    )
}

/// Snapshot the typed lifecycle records, optionally scoped to a `parent_session`.
#[cfg(test)]
pub(crate) fn task_records(parent_session: Option<&str>) -> Vec<OrchestrationTaskRecord> {
    let _ = task_store();
    let stores: Vec<Arc<dyn TaskStore>> = task_stores().values().unwrap_or_default();
    let all: Vec<OrchestrationTaskRecord> = stores
        .into_iter()
        .flat_map(|store| store.list(OrchestrationTaskFilter::default()))
        .collect();
    log::trace!(
        "[running_subagents] task_records loaded records={} parent_session_filter={:?}",
        all.len(),
        parent_session
    );
    match parent_session {
        Some(ps) => all
            .into_iter()
            .filter(|r| r.spec.metadata.get("parentSession").map(String::as_str) == Some(ps))
            .collect(),
        None => all,
    }
}
