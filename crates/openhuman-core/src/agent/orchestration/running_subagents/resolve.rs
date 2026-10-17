//! Resolving a durable `subagent_session_id` (or a transient `task_id`) back
//! to the live registry entry or, failing that, the durable task-store record
//! — used by tool/control paths that only know the stable session id a
//! sub-agent is addressed by.

use std::path::Path;

use tinyagents_orchestration::subagent::{
    resume_ref_for_task as live_resume_ref, resume_ref_from_record,
    task_id_for_session as live_task_id, task_id_for_session_in_records, SubagentResumeRef,
    WaitError,
};

use super::registry::{admit_workspace, owner_key, registry};
use super::task_ledger::{list_task_records, task_record_for_task_in_workspace};

/// Resolve a durable `subagent_session_id` to the currently-running transient
/// `task_id`, enforcing parent-session ownership.
pub(crate) fn task_id_for_session(
    ctx: &crate::core::runtime::DispatchCtx,
    subagent_session_id: &str,
    parent_session: &str,
) -> Result<String, WaitError> {
    live_task_id(
        registry(),
        subagent_session_id,
        &owner_key(ctx, parent_session)?,
    )
}

pub(crate) fn task_id_for_session_in_workspace(
    ctx: &crate::core::runtime::DispatchCtx,
    subagent_session_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
) -> Result<String, WaitError> {
    admit_workspace(ctx, workspace_dir)?;
    match task_id_for_session(ctx, subagent_session_id, parent_session) {
        Ok(task_id) => return Ok(task_id),
        Err(WaitError::NotOwned) => return Err(WaitError::NotOwned),
        Err(WaitError::RegistryPoisoned) => return Err(WaitError::RegistryPoisoned),
        Err(WaitError::Unknown) => {}
    }

    let task_id = task_id_for_session_in_records(
        list_task_records(workspace_dir),
        subagent_session_id,
        parent_session,
    )?;
    log::debug!(
        "[running_subagents] resolved session from task store subagent_session_id={} task_id={} workspace_dir={}",
        subagent_session_id,
        task_id,
        workspace_dir.display()
    );
    Ok(task_id)
}

pub(crate) fn resume_ref_for_task(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    parent_session: &str,
) -> Result<SubagentResumeRef, WaitError> {
    live_resume_ref(registry(), task_id, &owner_key(ctx, parent_session)?)
}

pub(crate) fn resume_ref_for_task_in_workspace(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
) -> Result<SubagentResumeRef, WaitError> {
    admit_workspace(ctx, workspace_dir)?;
    match resume_ref_for_task(ctx, task_id, parent_session) {
        Ok(reference) => return Ok(reference),
        Err(WaitError::NotOwned) => return Err(WaitError::NotOwned),
        Err(WaitError::RegistryPoisoned) => return Err(WaitError::RegistryPoisoned),
        Err(WaitError::Unknown) => {}
    }

    let record = task_record_for_task_in_workspace(workspace_dir, task_id, parent_session)?;
    log::debug!(
        "[running_subagents] resolved resume ref from task store task_id={} workspace_dir={}",
        task_id,
        workspace_dir.display()
    );
    Ok(resume_ref_from_record(task_id, &record))
}
