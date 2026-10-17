//! Aborting sub-agents: by task id (the user-facing "Cancel" affordance), by
//! durable session id, by owning parent thread (thread deletion), or all at
//! once (a full thread purge).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tinyagents_harness::ids::TaskId;

use tinyagents_orchestration::subagent::FinishedOutcome;

use super::registry::{control_admits, registry};
use super::resolve::task_id_for_session_in_workspace;
use super::task_ledger::record_cancelled;

/// Metadata captured when a sub-agent is cancelled, so the caller can surface
/// the cancellation back in the parent chat (record a "cancelled" completion
/// for idle-gated delivery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CancelledSubagent {
    pub(crate) agent_id: String,
    pub(crate) parent_session: String,
    pub(crate) subagent_session_id: Option<String>,
    pub(crate) workspace_dir: PathBuf,
    pub(crate) parent_thread_id: Option<String>,
    /// How the run had already ended when the cancel arrived, if it had. A
    /// finished entry stays registered until `sweep_terminal`, so a late
    /// "Cancel" still finds it; the caller must not rewrite that outcome as a
    /// user cancellation, and reports it instead.
    pub(crate) already_finished: Option<FinishedOutcome>,
}

/// Abort and drop the sub-agent with `task_id`, returning its metadata so the
/// caller can deliver a "cancelled" notice into the parent chat. Returns `None`
/// if no such sub-agent is registered (already finished, or unknown id).
///
/// User controls check the captured profile in SaaS. The desktop user retains
/// authority over every sub-agent in their own core.
pub(crate) fn cancel_by_task(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
) -> Option<CancelledSubagent> {
    let id = TaskId::new(task_id);
    let snapshot = registry().snapshot_trusted(&id).ok()?;
    if !control_admits(ctx, &snapshot.metadata) {
        return None;
    }
    let cancelled = registry().cancel(&id, &snapshot.owner_id).ok()?;
    // `AwaitingUser` is paused, not finished: cancelling it is a real cancel.
    let already_finished = cancelled.status.finished_outcome();
    let metadata = cancelled.metadata;
    if already_finished.is_none() {
        record_cancelled(&metadata.workspace_dir, task_id);
    }
    log::debug!(
        "[running_subagents] cancel_by_task task_id={} agent_id={} parent_thread_id={:?} live_entries={}",
        task_id,
        metadata.agent_id,
        metadata.parent_thread_id,
        registry()
            .len()
            .expect("detached task registry lock poisoned")
    );
    Some(CancelledSubagent {
        agent_id: metadata.agent_id,
        parent_session: metadata.parent_session,
        subagent_session_id: metadata.subagent_session_id,
        workspace_dir: metadata.workspace_dir,
        parent_thread_id: metadata.parent_thread_id,
        already_finished,
    })
}

pub(crate) fn cancel_by_session_in_workspace(
    ctx: &crate::core::runtime::DispatchCtx,
    subagent_session_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
) -> Option<CancelledSubagent> {
    let task_id =
        task_id_for_session_in_workspace(ctx, subagent_session_id, parent_session, workspace_dir)
            .ok()?;
    cancel_by_task(ctx, &task_id)
}

/// Abort and drop every running sub-agent whose parent chat thread is
/// `thread_id`. Called when that thread is deleted so detached children don't
/// keep running (and later try to deliver) against a thread that no longer
/// exists. Returns the number of sub-agents cancelled.
pub(crate) fn cancel_for_thread(ctx: &crate::core::runtime::DispatchCtx, thread_id: &str) -> usize {
    let own = caller_workspace(ctx);
    let cancelled = registry()
        .cancel_where(|metadata| {
            metadata.parent_thread_id.as_deref() == Some(thread_id) && own.admits(ctx, metadata)
        })
        .expect("detached task registry lock poisoned");
    for entry in &cancelled {
        record_cancelled(&entry.metadata.workspace_dir, entry.task_id.as_str());
    }
    let count = cancelled.len();
    log::debug!(
        "[running_subagents] cancel_for_thread thread_id={} cancelled={} live_entries={}",
        thread_id,
        count,
        registry()
            .len()
            .expect("detached task registry lock poisoned")
    );
    count
}

/// Abort every running detached sub-agent spawned from chat thread
/// `thread_id` because the user pressed Stop on that thread.
///
/// Unlike [`cancel_for_thread`] (thread deletion) the thread survives, so each
/// child's durable sub-agent session is marked failed ("cancelled by user")
/// rather than left looking resumable. No "you cancelled" completion is
/// recorded: delivering one would start a fresh system turn on the thread,
/// which is exactly what Stop is meant to prevent. Returns the cancelled task
/// ids.
pub(crate) fn stop_for_thread(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
) -> Vec<String> {
    let own = caller_workspace(ctx);
    let cancelled = registry()
        .cancel_where(|metadata| {
            metadata.parent_thread_id.as_deref() == Some(thread_id) && own.admits(ctx, metadata)
        })
        .expect("detached task registry lock poisoned");
    let mut task_ids = Vec::with_capacity(cancelled.len());
    for entry in cancelled {
        let task_id = entry.task_id.as_str().to_string();
        record_cancelled(&entry.metadata.workspace_dir, &task_id);
        if let Some(subagent_session_id) = entry.metadata.subagent_session_id.as_deref() {
            let store = crate::agent::orchestration::subagent_sessions::SubagentSessionStore::new(
                entry.metadata.workspace_dir.clone(),
            );
            if let Err(err) = crate::agent::orchestration::subagent_sessions::mark_failed(
                &store,
                subagent_session_id,
                &task_id,
                "cancelled by user".to_string(),
            ) {
                log::warn!(
                    "[running_subagents] stop_for_thread mark_failed failed thread_id={} task_id={} subagent_session_id={} error={}",
                    thread_id,
                    task_id,
                    subagent_session_id,
                    err
                );
            }
        }
        task_ids.push(task_id);
    }
    log::info!(
        "[running_subagents] stop_for_thread thread_id={} cancelled={} live_entries={}",
        thread_id,
        task_ids.len(),
        registry()
            .len()
            .expect("detached task registry lock poisoned")
    );
    task_ids
}

/// Abort and drop **every** registered sub-agent. Called on a full thread purge
/// where no parent thread survives. Returns the **distinct parent thread ids**
/// that had sub-agents, so the purge path can tombstone them in
/// [`super::super::background_completions`] and drop any straggler completion
/// that wins the cooperative-abort race. Headless sub-agents (no parent thread)
/// are still aborted but contribute no id.
pub(crate) fn cancel_all(ctx: &crate::core::runtime::DispatchCtx) -> Vec<String> {
    let own = caller_workspace(ctx);
    let cancelled = registry()
        .cancel_where(|metadata| own.admits(ctx, metadata))
        .expect("detached task registry lock poisoned");
    let count = cancelled.len();
    let mut thread_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for entry in cancelled {
        record_cancelled(&entry.metadata.workspace_dir, entry.task_id.as_str());
        if let Some(thread_id) = entry.metadata.parent_thread_id {
            if seen.insert(thread_id.clone()) {
                thread_ids.push(thread_id);
            }
        }
    }
    log::debug!(
        "[running_subagents] cancel_all cancelled={} distinct_threads={}",
        count,
        thread_ids.len()
    );
    thread_ids
}

/// Which registered sub-agents a thread-scoped cancel may reach.
///
/// Thread ids are only unique per user, and the registry is process-wide. On
/// the desktop every child belongs to the one user, as before. In SaaS only
/// children working in the caller's own (profile) workspace qualify, so one
/// user's Stop, delete or purge never reaches another user's work; a caller
/// with no scope reaches none.
pub(crate) enum CallerWorkspace {
    All,
    Only(std::path::PathBuf),
    Nothing,
}

impl CallerWorkspace {
    pub(crate) fn admits(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        metadata: &super::registry::RunningSubagentMetadata,
    ) -> bool {
        match self {
            Self::All => true,
            Self::Only(dir) => metadata.workspace_dir == *dir && control_admits(ctx, metadata),
            Self::Nothing => false,
        }
    }
}

pub(crate) fn caller_workspace(ctx: &crate::core::runtime::DispatchCtx) -> CallerWorkspace {
    caller_workspace_in(crate::core::runtime::is_saas(), ctx.resolved().as_deref())
}

/// [`caller_workspace`] as a function of the mode and the task's own scope.
pub(crate) fn caller_workspace_in(
    saas: bool,
    scoped: Option<&crate::core::runtime::CoreContext>,
) -> CallerWorkspace {
    if !saas {
        return CallerWorkspace::All;
    }
    match scoped.and_then(|ctx| ctx.workspace_dir().ok()) {
        Some(dir) => CallerWorkspace::Only(dir),
        None => {
            log::warn!("[running_subagents] cancel without a tenant scope in SaaS; reaching none");
            CallerWorkspace::Nothing
        }
    }
}

#[cfg(test)]
pub(crate) fn prune(task_id: &str) {
    let _ = registry().cancel_trusted(&TaskId::new(task_id));
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
