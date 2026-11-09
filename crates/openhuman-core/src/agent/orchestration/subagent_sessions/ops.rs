use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::agent::subagent_host::SubagentRunStatus;
use tinyagents_session::transcript::TranscriptMessage;

use super::types::{
    DurableSubagentSession, DurableSubagentStatus, ReuseDecision, SubagentSessionSelector,
    SubagentSessionStore, SubagentSessionUpsert,
};

#[path = "ops_driver.rs"]
mod driver;
#[cfg(test)]
use driver::record_cancel_reason;
#[cfg(test)]
pub(crate) use driver::{cancel_driver_job_by_task, mark_driver_pause_cancelled};
pub use driver::{
    cancel_driver_job_for_observed_session, cancel_driver_jobs_for_thread,
    resolve_driver_lifecycle_key, resolve_lifecycle_key, DriverTaskCancellation,
};
pub use tinyagents_orchestration::subagent::normalize_task_key;

pub fn action_root_key(path: Option<&Path>) -> Option<String> {
    path.map(|p| p.to_string_lossy().to_string())
}

pub fn find_reusable(
    store: &SubagentSessionStore,
    selector: &SubagentSessionSelector,
) -> Result<Option<DurableSubagentSession>, String> {
    let mut matches: Vec<DurableSubagentSession> = store
        .load()?
        .into_iter()
        .filter(|session| session.matches_selector(selector))
        .collect();
    matches.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
    Ok(matches.into_iter().next())
}

pub fn upsert_running(
    store: &SubagentSessionStore,
    upsert: SubagentSessionUpsert,
    reuse: Option<&DurableSubagentSession>,
) -> Result<DurableSubagentSession, String> {
    let _guard = store_write_lock()
        .lock()
        .map_err(|_| "subagent session store lock poisoned".to_string())?;
    if let Some(reuse) = reuse {
        let selector = upsert.selector.clone();
        let worker_thread_id = upsert.worker_thread_id.clone();
        let display_name = upsert.display_name.clone();
        let task_id = upsert.task_id.clone();
        let lifecycle_key = upsert.lifecycle_key.clone();
        let expected_task_id = reuse.current_task_id.clone();
        let expected_lifecycle_key = reuse.lifecycle_key.clone();
        if let Some(updated) =
            store.update_session_if(&reuse.subagent_session_id, move |session, now| {
                if session.status == DurableSubagentStatus::Closed
                    || session.current_task_id != expected_task_id
                    || session.lifecycle_key != expected_lifecycle_key
                    || !session.matches_selector(&selector)
                {
                    return false;
                }
                session.parent_session = selector.parent_session.clone();
                session.parent_thread_id = selector.parent_thread_id.clone();
                session.worker_thread_id = worker_thread_id
                    .clone()
                    .or_else(|| session.worker_thread_id.clone());
                session.display_name = display_name
                    .clone()
                    .or_else(|| session.display_name.clone());
                session.current_task_id = Some(task_id.clone());
                if let Some(lifecycle_key) = &lifecycle_key {
                    session.lifecycle_key = Some(lifecycle_key.clone());
                }
                session.parent_epoch = upsert.parent_epoch;
                session.status = DurableSubagentStatus::Running;
                session.reusable = true;
                session.latest_error = None;
                session.updated_at = now.clone();
                session.last_used_at = now;
                true
            })?
        {
            return Ok(updated);
        }
        if store
            .load()?
            .iter()
            .any(|session| session.subagent_session_id == reuse.subagent_session_id)
        {
            return Err("stale reusable sub-agent session changed during upsert".into());
        }
    }
    let mut sessions = store.load()?;
    let now = chrono::Utc::now().to_rfc3339();
    let session_id = format!("subsess-{}", uuid::Uuid::new_v4());

    let session = DurableSubagentSession {
        subagent_session_id: session_id.clone(),
        parent_session: upsert.selector.parent_session.clone(),
        parent_thread_id: upsert.selector.parent_thread_id.clone(),
        worker_thread_id: upsert.worker_thread_id.clone(),
        agent_id: upsert.selector.agent_id.clone(),
        display_name: upsert.display_name.clone(),
        model: upsert.selector.model.clone(),
        sandbox_mode: upsert.selector.sandbox_mode.clone(),
        action_root: upsert.selector.action_root.clone(),
        task_key: upsert.selector.task_key.clone(),
        task_title: upsert.task_title.clone(),
        current_task_id: Some(upsert.task_id),
        lifecycle_key: upsert.lifecycle_key,
        parent_epoch: upsert.parent_epoch,
        status: DurableSubagentStatus::Running,
        reusable: true,
        latest_history: None,
        latest_error: None,
        created_at: now.clone(),
        updated_at: now.clone(),
        last_used_at: now.clone(),
    };

    sessions.retain(|existing| existing.subagent_session_id != session_id);
    sessions.push(session.clone());
    store.save(&sessions)?;
    Ok(session)
}

pub fn mark_finished(
    store: &SubagentSessionStore,
    subagent_session_id: &str,
    task_id: &str,
    run_status: &SubagentRunStatus,
    history: Vec<TranscriptMessage>,
) -> Result<(), String> {
    let task_id = task_id.to_owned();
    let task_id_for_update = task_id.clone();
    let status = DurableSubagentStatus::from_run_status(run_status);
    let updated =
        update_session_for_task(store, subagent_session_id, &task_id, move |session, now| {
            session.current_task_id = Some(task_id_for_update.clone());
            session.status = status;
            session.latest_history = Some(history.clone());
            session.latest_error = None;
            session.updated_at = now.clone();
            session.last_used_at = now;
        })?;
    if updated {
        Ok(())
    } else {
        Err(format!(
            "sub-agent session not found: {subagent_session_id}"
        ))
    }
}

pub fn mark_failed(
    store: &SubagentSessionStore,
    subagent_session_id: &str,
    task_id: &str,
    error: String,
) -> Result<(), String> {
    let task_id = task_id.to_owned();
    let task_id_for_update = task_id.clone();
    let updated =
        update_session_for_task(store, subagent_session_id, &task_id, move |session, now| {
            session.current_task_id = Some(task_id_for_update.clone());
            session.status = DurableSubagentStatus::Failed;
            session.latest_error = Some(error.clone());
            session.updated_at = now.clone();
            session.last_used_at = now;
        })?;
    if updated {
        Ok(())
    } else {
        Err(format!(
            "sub-agent session not found: {subagent_session_id}"
        ))
    }
}

pub fn close(store: &SubagentSessionStore, subagent_session_id: &str) -> Result<bool, String> {
    update_session(store, subagent_session_id, |session, now| {
        session.status = DurableSubagentStatus::Closed;
        session.reusable = false;
        session.updated_at = now.clone();
        session.last_used_at = now;
    })
}

/// Closes a session only if it still contains the lifecycle snapshot that the
/// caller observed and cancelled. `false` means a concurrent reuse or close
/// won the race and this stale request did not alter the newer row.
pub fn close_if_current(
    store: &SubagentSessionStore,
    observed: &DurableSubagentSession,
) -> Result<bool, String> {
    let _guard = store_write_lock()
        .lock()
        .map_err(|_| "subagent session store lock poisoned".to_string())?;
    let observed = observed.clone();
    Ok(store
        .update_session_if(&observed.subagent_session_id, move |session, now| {
            if session.parent_session != observed.parent_session
                || session.parent_thread_id != observed.parent_thread_id
                || session.current_task_id != observed.current_task_id
                || session.lifecycle_key != observed.lifecycle_key
            {
                return false;
            }
            session.status = DurableSubagentStatus::Closed;
            session.reusable = false;
            session.updated_at = now.clone();
            session.last_used_at = now;
            true
        })?
        .is_some())
}

pub fn list_for_parent(
    store: &SubagentSessionStore,
    parent_session: &str,
    parent_thread_id: Option<&str>,
) -> Result<Vec<DurableSubagentSession>, String> {
    let mut sessions: Vec<DurableSubagentSession> = store
        .load()?
        .into_iter()
        .filter(|session| {
            session.parent_session == parent_session
                && parent_thread_id
                    .map(|thread_id| session.parent_thread_id.as_deref() == Some(thread_id))
                    .unwrap_or(true)
        })
        .collect();
    sessions.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
    Ok(sessions)
}

pub fn reuse_decision(reuse: Option<&DurableSubagentSession>, force_fresh: bool) -> ReuseDecision {
    if force_fresh {
        return ReuseDecision::ForcedFresh;
    }
    match reuse.map(|session| session.status) {
        Some(DurableSubagentStatus::Running) => ReuseDecision::ReusedRunning,
        Some(_) => ReuseDecision::ReusedIdle,
        None => ReuseDecision::SpawnedNew,
    }
}

fn update_session<F>(
    store: &SubagentSessionStore,
    subagent_session_id: &str,
    update: F,
) -> Result<bool, String>
where
    F: FnMut(&mut DurableSubagentSession, String) + Send + 'static,
{
    let _guard = store_write_lock()
        .lock()
        .map_err(|_| "subagent session store lock poisoned".to_string())?;
    store
        .update_session(subagent_session_id, update)
        .map(|updated| updated.is_some())
}

/// Apply a terminal write only while it still belongs to the current task.
/// The check and write share the store lock so a late completion cannot
/// overwrite a reused session or revive one that has already been closed.
fn update_session_for_task<F>(
    store: &SubagentSessionStore,
    subagent_session_id: &str,
    task_id: &str,
    mut update: F,
) -> Result<bool, String>
where
    F: FnMut(&mut DurableSubagentSession, String) + Send + 'static,
{
    let _guard = store_write_lock()
        .lock()
        .map_err(|_| "subagent session store lock poisoned".to_string())?;
    let task_id = task_id.to_owned();
    Ok(store
        .update_session_if(subagent_session_id, move |session, now| {
            if session.status == DurableSubagentStatus::Closed
                || session.current_task_id.as_deref() != Some(task_id.as_str())
            {
                return false;
            }
            update(session, now);
            true
        })?
        .is_some())
}

fn store_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
