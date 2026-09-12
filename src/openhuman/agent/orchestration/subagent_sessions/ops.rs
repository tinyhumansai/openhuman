use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use crate::openhuman::agent::harness::subagent_runner::SubagentRunStatus;
use crate::openhuman::agent::messages::ChatMessage;

use super::types::{
    DurableSubagentSession, DurableSubagentStatus, ReuseDecision, SubagentSessionSelector,
    SubagentSessionStore, SubagentSessionUpsert,
};

pub fn normalize_task_key(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.trim().to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        if input.trim().is_empty() {
            "untitled-task".to_string()
        } else {
            format!("task-{:016x}", task_key_hash(input))
        }
    } else if out.len() > 96 {
        let hash = format!("{:016x}", task_key_hash(input));
        let mut prefix = String::new();
        for ch in out.chars() {
            if prefix.len() + ch.len_utf8() + hash.len() + 1 > 96 {
                break;
            }
            prefix.push(ch);
        }
        while prefix.ends_with('-') {
            prefix.pop();
        }
        format!("{prefix}-{hash}")
    } else {
        out
    }
}

fn task_key_hash(input: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    hasher.finish()
}

pub fn task_title_from_prompt(prompt: &str) -> String {
    let collapsed = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= 80 {
        collapsed
    } else {
        let mut title: String = collapsed.chars().take(77).collect();
        title.push_str("...");
        title
    }
}

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
    let mut sessions = store.load()?;
    let now = chrono::Utc::now().to_rfc3339();
    let session_id = reuse
        .map(|session| session.subagent_session_id.clone())
        .unwrap_or_else(|| format!("subsess-{}", uuid::Uuid::new_v4()));

    let mut session = if let Some(existing) = sessions
        .iter()
        .find(|session| session.subagent_session_id == session_id)
        .cloned()
    {
        existing
    } else {
        DurableSubagentSession {
            subagent_session_id: session_id.clone(),
            parent_session: upsert.selector.parent_session.clone(),
            parent_thread_id: upsert.selector.parent_thread_id.clone(),
            worker_thread_id: upsert.worker_thread_id.clone(),
            agent_id: upsert.selector.agent_id.clone(),
            display_name: upsert.display_name.clone(),
            toolkit: upsert.selector.toolkit.clone(),
            model: upsert.selector.model.clone(),
            sandbox_mode: upsert.selector.sandbox_mode.clone(),
            action_root: upsert.selector.action_root.clone(),
            task_key: upsert.selector.task_key.clone(),
            task_title: upsert.task_title.clone(),
            current_task_id: None,
            status: DurableSubagentStatus::Running,
            reusable: true,
            latest_history: None,
            latest_error: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_used_at: now.clone(),
        }
    };

    session.parent_session = upsert.selector.parent_session;
    session.parent_thread_id = upsert.selector.parent_thread_id;
    session.worker_thread_id = upsert.worker_thread_id.or(session.worker_thread_id);
    session.display_name = upsert.display_name.or(session.display_name);
    session.current_task_id = Some(upsert.task_id);
    session.status = DurableSubagentStatus::Running;
    session.reusable = true;
    session.latest_error = None;
    session.updated_at = now.clone();
    session.last_used_at = now;

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
    history: Vec<ChatMessage>,
) -> Result<(), String> {
    let updated = update_session(store, subagent_session_id, |session, now| {
        session.current_task_id = Some(task_id.to_string());
        session.status = DurableSubagentStatus::from_run_status(run_status);
        session.latest_history = Some(history);
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
    let updated = update_session(store, subagent_session_id, |session, now| {
        session.current_task_id = Some(task_id.to_string());
        session.status = DurableSubagentStatus::Failed;
        session.latest_error = Some(error);
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
    F: FnOnce(&mut DurableSubagentSession, String),
{
    let _guard = store_write_lock()
        .lock()
        .map_err(|_| "subagent session store lock poisoned".to_string())?;
    let mut sessions = store.load()?;
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(session) = sessions
        .iter_mut()
        .find(|session| session.subagent_session_id == subagent_session_id)
    {
        update(session, now);
        store.save(&sessions)?;
        return Ok(true);
    }
    Ok(false)
}

fn store_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
