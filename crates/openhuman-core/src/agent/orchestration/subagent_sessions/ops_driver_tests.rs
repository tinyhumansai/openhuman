use super::*;

const FIXED_TIMESTAMP: &str = "2026-10-19T12:00:00.000Z";

fn session(updated_at: &str) -> DurableSubagentSession {
    DurableSubagentSession {
        subagent_session_id: "session-1".into(),
        parent_session: "parent-1".into(),
        parent_thread_id: Some("thread-1".into()),
        parent_epoch: None,
        worker_thread_id: Some("worker-1".into()),
        agent_id: "researcher".into(),
        display_name: None,
        model: Some("model-1".into()),
        sandbox_mode: "read_only".into(),
        action_root: None,
        task_key: "task-key".into(),
        task_title: "Task title".into(),
        current_task_id: Some("task-1".into()),
        lifecycle_key: Some(tinyagents_orchestration::subagent::SubagentTaskKey {
            root_run_id: "root-1".into(),
            parent_run_id: "parent-run-1".into(),
            thread_id: Some("thread-1".into()),
            task_id: "task-1".into(),
        }),
        status: DurableSubagentStatus::Running,
        reusable: true,
        latest_history: None,
        latest_error: None,
        created_at: updated_at.into(),
        updated_at: updated_at.into(),
        last_used_at: updated_at.into(),
    }
}

#[test]
fn observed_mutation_persists_when_the_timestamp_does_not_advance() {
    let dir = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(dir.path().to_path_buf());
    let observed = session(FIXED_TIMESTAMP);
    store.save(std::slice::from_ref(&observed)).unwrap();

    let updated = mutate_observed_session(&store, &observed, |current, _now| {
        // This models the equal-millisecond case without sleeping or relying
        // on clock scheduling: the requested field changes, but its persisted
        // updated_at remains exactly the stored timestamp.
        current.latest_error = Some("cancel reason".into());
    })
    .unwrap();

    assert!(updated);
    let persisted = store.load().unwrap().pop().unwrap();
    assert_eq!(persisted.updated_at, FIXED_TIMESTAMP);
    assert_eq!(persisted.latest_error.as_deref(), Some("cancel reason"));
}

fn change_parent(session: &mut DurableSubagentSession) {
    session.parent_session = "replacement-parent".into();
}

fn change_thread(session: &mut DurableSubagentSession) {
    session.parent_thread_id = Some("replacement-thread".into());
}

fn change_task(session: &mut DurableSubagentSession) {
    session.current_task_id = Some("replacement-task".into());
}

fn change_lifecycle(session: &mut DurableSubagentSession) {
    session.lifecycle_key.as_mut().unwrap().root_run_id = "replacement-root".into();
}

fn change_status(session: &mut DurableSubagentSession) {
    session.status = DurableSubagentStatus::AwaitingUser;
}

#[test]
fn observed_mutation_rejects_a_stale_parent_thread_task_lifecycle_or_status() {
    type SessionChange = fn(&mut DurableSubagentSession);
    let changes: [(&str, SessionChange); 5] = [
        ("parent", change_parent),
        ("thread", change_thread),
        ("task", change_task),
        ("lifecycle", change_lifecycle),
        ("status", change_status),
    ];

    for (field, change_current) in changes {
        let dir = tempfile::tempdir().unwrap();
        let store = SubagentSessionStore::new(dir.path().to_path_buf());
        let observed = session(FIXED_TIMESTAMP);
        let mut current = observed.clone();
        change_current(&mut current);
        store.save(std::slice::from_ref(&current)).unwrap();

        let closure_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let closure_marker = std::sync::Arc::clone(&closure_called);
        let updated = mutate_observed_session(&store, &observed, move |session, _now| {
            closure_marker.store(true, std::sync::atomic::Ordering::SeqCst);
            session.latest_error = Some("stale mutation".into());
        })
        .unwrap();

        assert!(!updated, "stale {field} must not update the current row");
        assert!(
            !closure_called.load(std::sync::atomic::Ordering::SeqCst),
            "stale {field} must reject before mutation"
        );
        let persisted = store.load().unwrap().pop().unwrap();
        assert_eq!(
            serde_json::to_value(&persisted).unwrap(),
            serde_json::to_value(&current).unwrap(),
            "stale {field} must leave the current row unchanged"
        );
    }
}
