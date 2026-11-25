use std::path::Path;

use super::*;

/// Requests cancellation of driver-backed children owned by a thread being
/// removed. The TinyAgents registry owns cancellation and settlement; this
/// adapter only selects durable rows and resolves their authenticated keys.
pub fn cancel_driver_jobs_for_thread(
    store: &SubagentSessionStore,
    workspace_dir: &Path,
    jobs: &tinyagents_orchestration::subagent::SubAgentJobRegistry,
    thread_id: &str,
) -> Vec<String> {
    let sessions = match store.load() {
        Ok(sessions) => sessions,
        Err(error) => {
            log::warn!(
                "[subagent_sessions] failed to load sessions for thread cancellation: {error}"
            );
            return Vec::new();
        }
    };
    let mut cancelled = Vec::new();
    for session in sessions.into_iter().filter(|session| {
        session.parent_thread_id.as_deref() == Some(thread_id)
            && matches!(
                session.status,
                DurableSubagentStatus::Running | DurableSubagentStatus::AwaitingUser
            )
    }) {
        let Some(task_id) = session.current_task_id.as_deref() else {
            continue;
        };
        let key = session
            .lifecycle_key
            .filter(|key| key.task_id == task_id)
            .or_else(|| {
                resolve_lifecycle_key(
                    store,
                    &session.subagent_session_id,
                    &session.parent_session,
                    Some(thread_id),
                    &workspace_dir.join(".openhuman/subagent_checkpoints"),
                )
                .ok()
                .filter(|key| key.task_id == task_id)
            });
        let Some(key) = key else {
            continue;
        };
        let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
            workspace_dir,
            &session.parent_session,
            Some(thread_id),
        );
        if jobs.cancel_key_owned(&key, owner).is_ok() {
            cancelled.push(task_id.to_owned());
        }
    }
    cancelled
}

/// Requests cancellation for the exact durable row observed by the caller.
/// Keeping this separate from a fresh session-id lookup prevents concurrent
/// reuse from redirecting cancellation to a replacement child.
pub fn cancel_driver_job_for_observed_session(
    store: &SubagentSessionStore,
    checkpoint_dir: &Path,
    jobs: &tinyagents_orchestration::subagent::SubAgentJobRegistry,
    session: &DurableSubagentSession,
    parent_session: &str,
    parent_thread_id: Option<&str>,
) -> bool {
    if session.parent_session != parent_session
        || session.parent_thread_id.as_deref() != parent_thread_id
    {
        return false;
    }
    let Some(task_id) = session.current_task_id.as_deref() else {
        return false;
    };
    let key = session
        .lifecycle_key
        .clone()
        .filter(|key| key.task_id == task_id && key.thread_id.as_deref() == parent_thread_id)
        .or_else(|| {
            resolve_lifecycle_key(
                store,
                &session.subagent_session_id,
                parent_session,
                parent_thread_id,
                checkpoint_dir,
            )
            .ok()
            .filter(|key| key.task_id == task_id)
        });
    let Some(key) = key else {
        return false;
    };
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &store.workspace_dir,
        parent_session,
        parent_thread_id,
    );
    jobs.cancel_key_owned(&key, owner).is_ok()
}

/// A driver-backed child selected and cancelled by its public task id.
///
/// The selector is scoped to the caller's durable session store and refuses
/// ambiguous task IDs. The actual cancellation remains keyed by the full
/// authenticated TinyAgents lifecycle key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverTaskCancellation {
    pub task_id: String,
    pub agent_id: String,
    pub parent_session: String,
    pub subagent_session_id: String,
    pub workspace_dir: std::path::PathBuf,
    pub parent_thread_id: Option<String>,
    pub parent_epoch: Option<u64>,
    pub already_finished: Option<tinyagents_orchestration::subagent::FinishedOutcome>,
    pub was_paused: bool,
    pub lifecycle_key: Option<tinyagents_orchestration::subagent::SubagentTaskKey>,
}

pub fn cancel_driver_job_by_task(
    store: &SubagentSessionStore,
    checkpoint_dir: &Path,
    jobs: &tinyagents_orchestration::subagent::SubAgentJobRegistry,
    task_id: &str,
    reason: Option<&str>,
) -> Result<Option<DriverTaskCancellation>, String> {
    let mut matches = store
        .load()
        .map_err(|error| format!("failed to load subagent sessions for cancellation: {error}"))?
        .into_iter()
        .filter(|session| session.current_task_id.as_deref() == Some(task_id));
    let Some(session) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Ok(None);
    }
    let was_paused = session.status == DurableSubagentStatus::AwaitingUser;
    if session.status == DurableSubagentStatus::Closed {
        return Ok(None);
    }
    let lifecycle_key = match session.lifecycle_key.clone() {
        Some(key) => Some(key),
        None if session.current_task_id.is_some()
            && matches!(
                session.status,
                DurableSubagentStatus::Idle
                    | DurableSubagentStatus::Running
                    | DurableSubagentStatus::AwaitingUser
            ) =>
        {
            Some(
                resolve_lifecycle_key(
                    store,
                    task_id,
                    &session.parent_session,
                    session.parent_thread_id.as_deref(),
                    checkpoint_dir,
                )
                .map_err(|error| {
                    format!("failed to resolve subagent lifecycle for cancellation: {error}")
                })?,
            )
        }
        None => None,
    };
    if lifecycle_key
        .as_ref()
        .is_some_and(|key| key.task_id != task_id || key.thread_id != session.parent_thread_id)
    {
        return Ok(None);
    }
    let typed_finished =
        || -> Result<Option<tinyagents_orchestration::subagent::FinishedOutcome>, String> {
            let Some(key) = lifecycle_key.as_ref() else {
                return Ok(None);
            };
            let Some(outcome) =
                crate::agent::subagent_host::load_terminal_outcome(&store.workspace_dir, key)?
            else {
                return Ok(None);
            };
            Ok(match outcome.status {
                tinyagents_orchestration::subagent::SubagentOutcomeKind::Completed => {
                    Some(tinyagents_orchestration::subagent::FinishedOutcome::Completed)
                }
                tinyagents_orchestration::subagent::SubagentOutcomeKind::Incomplete(_) => {
                    Some(tinyagents_orchestration::subagent::FinishedOutcome::Failed)
                }
                tinyagents_orchestration::subagent::SubagentOutcomeKind::Cancelled
                | tinyagents_orchestration::subagent::SubagentOutcomeKind::AwaitingInput(_) => None,
            })
        };
    let already_finished = match session.status {
        DurableSubagentStatus::Idle => {
            let finished = typed_finished()?;
            if finished.is_none() {
                return Ok(None);
            }
            finished
        }
        DurableSubagentStatus::Failed => match typed_finished()? {
            Some(finished) => Some(finished),
            // Pre-driver failures have a failed product row but no typed
            // lifecycle record. Keep that legacy outcome reportable.
            None if session.lifecycle_key.is_none() => {
                Some(tinyagents_orchestration::subagent::FinishedOutcome::Failed)
            }
            None => return Ok(None),
        },
        DurableSubagentStatus::Running | DurableSubagentStatus::AwaitingUser => None,
        DurableSubagentStatus::Closed => unreachable!("closed handled above"),
    };
    if already_finished.is_none()
        && reason.is_some()
        && !record_cancel_reason(store, &session, reason.expect("checked above").trim())?
    {
        return Ok(None);
    }
    if already_finished.is_none()
        && !cancel_driver_job_for_observed_session(
            store,
            checkpoint_dir,
            jobs,
            &session,
            &session.parent_session,
            session.parent_thread_id.as_deref(),
        )
    {
        if reason.is_some() {
            clear_cancel_reason(store, &session);
        }
        return Ok(None);
    }
    Ok(Some(DriverTaskCancellation {
        task_id: task_id.to_owned(),
        agent_id: session.agent_id,
        parent_session: session.parent_session,
        subagent_session_id: session.subagent_session_id,
        workspace_dir: store.workspace_dir.clone(),
        parent_thread_id: session.parent_thread_id,
        parent_epoch: session.parent_epoch,
        already_finished,
        was_paused,
        lifecycle_key,
    }))
}

/// Mark a paused driver's durable product row cancelled only if it is still
/// the same lifecycle selected by the RPC. Paused driver completion has
/// already returned, so there is no live callback left to project this host
/// state transition.
pub fn mark_driver_pause_cancelled(
    store: &SubagentSessionStore,
    observed: &DriverTaskCancellation,
) -> Result<bool, String> {
    if !observed.was_paused {
        return Ok(false);
    }
    let id = observed.subagent_session_id.clone();
    let parent_session = observed.parent_session.clone();
    let parent_thread_id = observed.parent_thread_id.clone();
    let task_id = observed.task_id.clone();
    let lifecycle_key = observed.lifecycle_key.clone();
    store
        .update_session_if(&id, move |session, now| {
            if session.parent_session != parent_session
                || session.parent_thread_id != parent_thread_id
                || session.current_task_id.as_deref() != Some(task_id.as_str())
                || session.lifecycle_key != lifecycle_key
                || session.status != DurableSubagentStatus::AwaitingUser
            {
                return false;
            }
            session.status = DurableSubagentStatus::Failed;
            session
                .latest_error
                .get_or_insert_with(|| "cancelled by user".into());
            session.updated_at = now.clone();
            session.last_used_at = now;
            true
        })
        .map(|updated| updated.is_some())
}

/// Persist an optional RPC cancellation reason before signaling TinyAgents so
/// the awaited completion callback can include it in the single cancellation
/// notice. The current task check prevents a stale cancel from annotating a
/// later reuse of the same durable session.
pub(crate) fn record_cancel_reason(
    store: &SubagentSessionStore,
    observed: &DurableSubagentSession,
    reason: &str,
) -> Result<bool, String> {
    if reason.is_empty() {
        return Ok(true);
    }
    let reason = reason.to_owned();
    mutate_observed_session(store, observed, move |session, now| {
        if matches!(
            session.status,
            DurableSubagentStatus::Running | DurableSubagentStatus::AwaitingUser
        ) {
            session.latest_error = Some(reason.clone());
            session.updated_at = now;
        }
    })
}

fn clear_cancel_reason(store: &SubagentSessionStore, observed: &DurableSubagentSession) {
    let _ = mutate_observed_session(store, observed, move |session, now| {
        session.latest_error = None;
        session.updated_at = now;
    });
}

fn mutate_observed_session<F>(
    store: &SubagentSessionStore,
    observed: &DurableSubagentSession,
    mut update: F,
) -> Result<bool, String>
where
    F: FnMut(&mut DurableSubagentSession, String) + Send + 'static,
{
    let observed = observed.clone();
    store
        .update_session_if(&observed.subagent_session_id, move |session, now| {
            if session.parent_session != observed.parent_session
                || session.parent_thread_id != observed.parent_thread_id
                || session.current_task_id != observed.current_task_id
                || session.lifecycle_key != observed.lifecycle_key
                || session.status != observed.status
            {
                return false;
            }
            update(session, now);
            true
        })
        .map(|updated| updated.is_some())
}

/// Resolve a saved product session to its exact TinyAgents lifecycle key.
///
/// The product session lookup is scoped by the authenticated parent session,
/// optional parent thread, and the workspace-specific store. Rows written
/// before `lifecycle_key` was added recover it only from the matching host
/// checkpoint; a public task id alone is never treated as a lifecycle key.
pub fn resolve_lifecycle_key(
    store: &SubagentSessionStore,
    session_or_task_id: &str,
    parent_session: &str,
    parent_thread_id: Option<&str>,
    checkpoint_dir: &Path,
) -> Result<tinyagents_orchestration::subagent::SubagentTaskKey, String> {
    let session = list_for_parent(store, parent_session, parent_thread_id)?
        .into_iter()
        .find(|session| {
            session.subagent_session_id == session_or_task_id
                || session.current_task_id.as_deref() == Some(session_or_task_id)
        })
        .ok_or_else(|| "durable subagent session not found".to_string())?;
    let task_id = session
        .current_task_id
        .as_deref()
        .ok_or_else(|| "durable subagent session has no current task".to_string())?;
    let key = if let Some(key) = session.lifecycle_key {
        key
    } else {
        crate::agent::subagent_host::load_subagent_checkpoint(checkpoint_dir, task_id)
            .map_err(|error| format!("legacy subagent checkpoint unavailable: {error}"))?
            .task_key
            .ok_or_else(|| "legacy subagent checkpoint has no lifecycle key".to_string())?
    };
    if key.task_id != task_id || key.thread_id.as_deref() != session.parent_thread_id.as_deref() {
        return Err("durable subagent lifecycle key does not match its owned session".into());
    }
    Ok(key)
}

/// Resolves and validates a task lifecycle key through the authenticated
/// TinyAgents session port. Product spawn/resume paths use this instead of
/// consulting the legacy host collection.
pub async fn resolve_driver_lifecycle_key(
    store: &dyn tinyagents_orchestration::subagent::SubagentSessionStore,
    session_or_task_id: &str,
    checkpoint_dir: &Path,
) -> Result<tinyagents_orchestration::subagent::SubagentTaskKey, String> {
    let session = store
        .get(session_or_task_id)
        .await
        .map_err(|error| format!("read durable subagent session: {error}"))?
        .ok_or_else(|| "durable subagent session not found".to_string())?;
    let task_id = session
        .current_task_id
        .as_deref()
        .ok_or_else(|| "durable subagent session has no current task".to_string())?;
    let key = if let Some(key) = session.lifecycle_key {
        key
    } else {
        crate::agent::subagent_host::load_subagent_checkpoint(checkpoint_dir, task_id)
            .map_err(|error| format!("legacy subagent checkpoint unavailable: {error}"))?
            .task_key
            .ok_or_else(|| "legacy subagent checkpoint has no lifecycle key".to_string())?
    };
    if key.task_id != task_id || key.thread_id.as_deref() != session.parent_thread_id.as_deref() {
        return Err("durable subagent lifecycle key does not match its owned session".into());
    }
    Ok(key)
}

#[cfg(test)]
#[path = "ops_driver_tests.rs"]
mod tests;
