use super::*;
use crate::agent::orchestration::subagent_sessions::types::SubagentSessionUpsert;

fn selector(task_key: &str) -> SubagentSessionSelector {
    SubagentSessionSelector {
        parent_session: "parent-a".into(),
        parent_thread_id: Some("thread-a".into()),
        agent_id: "researcher".into(),
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
        vec![TranscriptMessage::user("done")],
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

#[test]
fn legacy_session_with_toolkit_key_still_loads_and_reuses() {
    // Sessions persisted before the `toolkit` spawn argument was retired
    // carry a `toolkit` key. It must be ignored on load, and it must no
    // longer take part in reuse matching.
    let dir = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(dir.path().to_path_buf());
    let legacy = serde_json::json!([{
        "subagentSessionId": "subsess-legacy",
        "parentSession": "parent-a",
        "parentThreadId": "thread-a",
        "workerThreadId": null,
        "agentId": "researcher",
        "displayName": null,
        "toolkit": "gmail",
        "model": "oh-1",
        "sandboxMode": "read_only",
        "actionRoot": "/tmp/work",
        "taskKey": "legacy-task",
        "taskTitle": "Legacy task",
        "currentTaskId": null,
        "status": "idle",
        "reusable": true,
        "latestHistory": null,
        "latestError": null,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "lastUsedAt": "2026-01-01T00:00:00Z"
    }]);
    let parsed: Vec<DurableSubagentSession> =
        serde_json::from_value(legacy).expect("legacy toolkit key is ignored");
    store.save(&parsed).unwrap();

    let sessions = store.load().expect("legacy toolkit key is ignored");
    assert_eq!(sessions.len(), 1);
    let reused = find_reusable(&store, &selector("legacy-task"))
        .unwrap()
        .expect("legacy session reuses without a toolkit match");
    assert_eq!(reused.subagent_session_id, "subsess-legacy");
}
