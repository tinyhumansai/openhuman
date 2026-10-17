//! Blocking-with-timeout collection of a sub-agent's terminal result, with a
//! durable task-store fallback for a sub-agent this process never registered
//! in-memory (e.g. after a core restart).

use std::path::Path;
use std::time::Duration;

use tinyagents_orchestration::subagent::{
    record_to_wait_outcome, task_status_label, wait_detached, WaitError, WaitOutcome,
};

use super::registry::{admit_workspace, owner_key, registry};
use super::task_ledger::task_record_for_task_in_workspace;

/// Block until `task_id` reaches a terminal status or `timeout` elapses.
pub(crate) async fn wait(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    parent_session: &str,
    timeout: Duration,
) -> Result<WaitOutcome, WaitError> {
    wait_detached(
        registry(),
        task_id,
        &owner_key(ctx, parent_session)?,
        timeout,
    )
    .await
}

pub(crate) async fn wait_in_workspace(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
    timeout: Duration,
) -> Result<WaitOutcome, WaitError> {
    admit_workspace(ctx, workspace_dir)?;
    match wait(ctx, task_id, parent_session, timeout).await {
        Ok(outcome) => return Ok(outcome),
        Err(WaitError::NotOwned) => return Err(WaitError::NotOwned),
        Err(WaitError::RegistryPoisoned) => return Err(WaitError::RegistryPoisoned),
        Err(WaitError::Unknown) => {}
    }

    let record = task_record_for_task_in_workspace(workspace_dir, task_id, parent_session)?;
    log::debug!(
        "[running_subagents] resolved wait from task store task_id={} status={} workspace_dir={}",
        task_id,
        task_status_label(record.status),
        workspace_dir.display()
    );
    Ok(record_to_wait_outcome(record))
}
