use super::*;
use crate::openhuman::agent::orchestration::subagent_sessions::types::SubagentSessionUpsert;

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
