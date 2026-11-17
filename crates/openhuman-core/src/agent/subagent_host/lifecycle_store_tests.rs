use super::*;

fn key(task: &str) -> SubagentTaskKey {
    SubagentTaskKey {
        root_run_id: "root".into(),
        parent_run_id: "parent".into(),
        thread_id: Some("thread".into()),
        task_id: task.into(),
    }
}

fn pause_json(key: &SubagentTaskKey) -> serde_json::Value {
    serde_json::json!({
        "task_key": key,
        "task_id": key.task_id,
        "agent_id": "researcher",
        "history": [],
        "question": "which one?",
        "created_at": "2026-10-01T00:00:00Z",
        "output": "partial",
        "iterations": 2,
        "usage": {"input_tokens": 1, "output_tokens": 2, "cached_input_tokens": 0, "charged_amount_usd": 0.0},
        "artifact_paths": [],
        "elapsed_ms": 40
    })
}

#[test]
fn a_legacy_lifecycle_directory_is_imported_once_and_left_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let lifecycle = dir.path().join("lifecycle");
    std::fs::create_dir_all(&lifecycle).unwrap();
    let paused = key("task-a");
    let component = key_component(&paused);
    std::fs::write(
        lifecycle.join(format!("{component}.pause.json")),
        serde_json::to_vec_pretty(&pause_json(&paused)).unwrap(),
    )
    .unwrap();
    // The first lifecycle extraction wrote the index with a single `key`.
    std::fs::write(
        lifecycle.join(format!("task-{}.index.json", task_component("task-a"))),
        serde_json::to_vec(&serde_json::json!({"key": paused})).unwrap(),
    )
    .unwrap();

    let store = LifecycleStore::open(dir.path()).unwrap();
    let record = store.record(&paused).unwrap();
    assert_eq!(record.pause.unwrap().question, "which one?");
    assert_eq!(store.index_keys("task-a").unwrap(), vec![paused.clone()]);
    assert!(lifecycle.join(format!("{component}.pause.json")).exists());

    // A later change is not undone by reopening, which does not import again.
    store
        .update(&paused, |record| {
            record.pause = None;
            ((), true)
        })
        .unwrap();
    let reopened = LifecycleStore::open(dir.path()).unwrap();
    assert!(reopened.record(&paused).unwrap().pause.is_none());
}

#[test]
fn a_legacy_bare_terminal_outcome_is_read_as_a_terminal_record() {
    let dir = tempfile::tempdir().unwrap();
    let lifecycle = dir.path().join("lifecycle");
    std::fs::create_dir_all(&lifecycle).unwrap();
    let done = key("task-b");
    let mut outcome = SubagentOutcome::cancelled("task-b");
    outcome.output = "finished".into();
    std::fs::write(
        lifecycle.join(format!("{}.terminal.json", key_component(&done))),
        serde_json::to_vec(&outcome).unwrap(),
    )
    .unwrap();
    std::fs::write(lifecycle.join("notes.txt"), "not a lifecycle file").unwrap();

    let record = LifecycleStore::open(dir.path())
        .unwrap()
        .record(&done)
        .unwrap();
    let terminal = record.terminal.unwrap();
    assert_eq!(terminal.outcome.output, "finished");
    assert!(terminal.agent_id.is_empty());
}

#[test]
fn the_default_checkpoint_directory_belongs_to_its_workspace() {
    let workspace = std::path::Path::new("/w/space");
    assert_eq!(
        workspace_of(&workspace.join(".openhuman/subagent_checkpoints")),
        workspace
    );
    assert_eq!(
        workspace_of(std::path::Path::new("/x/y")),
        std::path::Path::new("/x/y")
    );
}

#[test]
fn an_oversized_key_component_is_hashed_so_distinct_keys_do_not_alias() {
    let long = |task: &str| SubagentTaskKey {
        root_run_id: "root".repeat(100),
        parent_run_id: "parent".repeat(100),
        thread_id: None,
        task_id: task.into(),
    };
    let (a, b) = (key_component(&long("a")), key_component(&long("b")));
    assert!(a.starts_with("sha256-") && a.len() < 80);
    assert_ne!(a, b);
}
