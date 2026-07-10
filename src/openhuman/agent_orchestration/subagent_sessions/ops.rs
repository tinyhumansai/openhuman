use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use crate::openhuman::agent::harness::subagent_runner::SubagentRunStatus;
use crate::openhuman::inference::provider::ChatMessage;

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
mod tests {
    use super::*;
    use crate::openhuman::agent_orchestration::subagent_sessions::types::SubagentSessionUpsert;

    fn selector(task_key: &str) -> SubagentSessionSelector {
        SubagentSessionSelector {
            parent_session: "parent-a".into(),
            parent_thread_id: Some("thread-a".into()),
            agent_id: "researcher".into(),
            toolkit: Some("github".into()),
            model: Some("oh-1".into()),
            sandbox_mode: "read_only".into(),
            action_root: Some("/tmp/work".into()),
            task_key: task_key.into(),
        }
    }

    #[test]
    fn normalize_task_key_is_deterministic() {
        assert_eq!(
            normalize_task_key("  Review: GitHub PR #123!! "),
            "review-github-pr-123"
        );
        assert_eq!(normalize_task_key("   "), "untitled-task");
    }

    #[test]
    fn normalize_task_key_preserves_non_latin_words() {
        assert_ne!(normalize_task_key("研究 caching"), "untitled-task");
        assert_ne!(
            normalize_task_key("研究 caching"),
            normalize_task_key("調査 caching")
        );
    }

    #[test]
    fn normalize_task_key_hashes_empty_or_long_colliding_slugs() {
        assert_ne!(normalize_task_key("🙂🙂🙂"), "untitled-task");
        let prefix = "research ".repeat(40);
        let first = normalize_task_key(&(prefix.clone() + "alpha"));
        let second = normalize_task_key(&(prefix + "beta"));
        assert!(first.len() <= 96, "{first}");
        assert!(second.len() <= 96, "{second}");
        assert_ne!(first, second);
    }

    #[test]
    fn compatible_session_reuses_and_incompatible_shape_spawns_new() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubagentSessionStore::new(dir.path().to_path_buf());
        let upsert = SubagentSessionUpsert {
            selector: selector("same-task"),
            display_name: Some("Researcher".into()),
            task_title: "Same task".into(),
            worker_thread_id: Some("worker-1".into()),
            task_id: "sub-1".into(),
        };
        let session = upsert_running(&store, upsert, None).unwrap();
        mark_finished(
            &store,
            &session.subagent_session_id,
            "sub-1",
            &SubagentRunStatus::Completed,
            vec![ChatMessage::user("done")],
        )
        .unwrap();

        let reusable = find_reusable(&store, &selector("same-task"))
            .unwrap()
            .expect("same selector reuses");
        assert_eq!(reusable.subagent_session_id, session.subagent_session_id);

        let mut different = selector("same-task");
        different.action_root = Some("/tmp/other".into());
        assert!(find_reusable(&store, &different).unwrap().is_none());
    }

    #[test]
    fn closed_session_is_not_reusable() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubagentSessionStore::new(dir.path().to_path_buf());
        let session = upsert_running(
            &store,
            SubagentSessionUpsert {
                selector: selector("task"),
                display_name: None,
                task_title: "Task".into(),
                worker_thread_id: None,
                task_id: "sub-1".into(),
            },
            None,
        )
        .unwrap();

        assert!(close(&store, &session.subagent_session_id).unwrap());
        assert!(find_reusable(&store, &selector("task")).unwrap().is_none());
    }

    #[test]
    fn list_for_parent_without_thread_id_does_not_filter_by_thread() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubagentSessionStore::new(dir.path().to_path_buf());
        let first = upsert_running(
            &store,
            SubagentSessionUpsert {
                selector: selector("first"),
                display_name: None,
                task_title: "First".into(),
                worker_thread_id: None,
                task_id: "sub-1".into(),
            },
            None,
        )
        .unwrap();

        let mut second_selector = selector("second");
        second_selector.parent_thread_id = Some("thread-b".into());
        let second = upsert_running(
            &store,
            SubagentSessionUpsert {
                selector: second_selector,
                display_name: None,
                task_title: "Second".into(),
                worker_thread_id: None,
                task_id: "sub-2".into(),
            },
            None,
        )
        .unwrap();

        let all = list_for_parent(&store, "parent-a", None).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|session| session.subagent_session_id == first.subagent_session_id));
        assert!(all
            .iter()
            .any(|session| session.subagent_session_id == second.subagent_session_id));

        let thread_a = list_for_parent(&store, "parent-a", Some("thread-a")).unwrap();
        assert_eq!(thread_a.len(), 1);
        assert_eq!(thread_a[0].subagent_session_id, first.subagent_session_id);
    }

    #[test]
    fn missing_session_updates_return_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubagentSessionStore::new(dir.path().to_path_buf());
        assert!(mark_finished(
            &store,
            "missing",
            "sub-1",
            &SubagentRunStatus::Completed,
            vec![]
        )
        .is_err());
        assert!(mark_failed(&store, "missing", "sub-1", "boom".into()).is_err());
        assert!(!close(&store, "missing").unwrap());
    }
}
