//! Unit tests for [`super::TurnStateStore`].

use super::*;
use crate::openhuman::threads::turn_state::types::{
    ToolTimelineEntry, ToolTimelineStatus, TurnLifecycle, TurnState,
};
use tempfile::tempdir;

fn sample_state(thread_id: &str) -> TurnState {
    TurnState::started(thread_id.to_string(), "req-1", 25, "2026-05-04T10:00:00Z")
}

#[test]
fn put_then_get_roundtrips_state() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut state = sample_state("thread-abc");
    state.lifecycle = TurnLifecycle::Streaming;
    state.iteration = 3;
    state.streaming_text = "hello".into();
    state.tool_timeline.push(ToolTimelineEntry {
        id: "tc-1".into(),
        name: "shell".into(),
        round: 1,
        status: ToolTimelineStatus::Running,
        args_buffer: Some("{".into()),
        display_name: None,
        detail: None,
        source_tool_name: None,
        subagent: None,
    });

    store.put(&state).expect("put");
    let loaded = store.get("thread-abc").expect("get").expect("present");
    assert_eq!(loaded, state);
}

#[test]
fn get_returns_none_when_absent() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    assert!(store.get("missing").expect("get").is_none());
}

#[test]
fn delete_removes_snapshot_and_reports_presence() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let state = sample_state("thread-x");
    store.put(&state).expect("put");
    assert!(store.delete("thread-x").expect("delete"));
    assert!(!store.delete("thread-x").expect("delete-again"));
    assert!(store.get("thread-x").expect("get").is_none());
}

#[test]
fn list_returns_every_snapshot() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    store.put(&sample_state("a")).expect("put a");
    store.put(&sample_state("b")).expect("put b");
    let mut ids: Vec<String> = store
        .list()
        .expect("list")
        .into_iter()
        .map(|s| s.thread_id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn list_on_missing_dir_is_empty() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn mark_all_interrupted_promotes_lifecycle_and_clears_active_fields() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut state = sample_state("t");
    state.lifecycle = TurnLifecycle::Streaming;
    state.active_tool = Some("shell".into());
    state.active_subagent = Some("researcher".into());
    store.put(&state).expect("put");

    let count = store
        .mark_all_interrupted("2026-05-04T10:01:00Z")
        .expect("mark");
    assert_eq!(count, 1);

    let loaded = store.get("t").expect("get").expect("present");
    assert_eq!(loaded.lifecycle, TurnLifecycle::Interrupted);
    assert_eq!(loaded.updated_at, "2026-05-04T10:01:00Z");
    assert!(loaded.active_tool.is_none());
    assert!(loaded.active_subagent.is_none());

    // Re-running is a no-op for already-interrupted snapshots.
    let count = store
        .mark_all_interrupted("2026-05-04T10:02:00Z")
        .expect("mark again");
    assert_eq!(count, 0);
}

#[test]
fn clear_all_removes_corrupted_snapshots_too() {
    use std::io::Write as _;
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    store.put(&sample_state("a")).expect("put a");
    store.put(&sample_state("b")).expect("put b");

    // Drop a corrupted JSON file alongside — `list()` would skip it,
    // but a destructive purge must still remove it.
    let corrupt_path = dir
        .path()
        .join("memory")
        .join("conversations")
        .join("turn_states")
        .join("deadbeef.json");
    let mut f = std::fs::File::create(&corrupt_path).expect("create corrupt");
    f.write_all(b"{ not valid json").expect("write corrupt");
    drop(f);
    assert!(corrupt_path.exists());

    let removed = store.clear_all().expect("clear_all");
    assert_eq!(removed, 3, "all three snapshots must be removed");
    assert!(!corrupt_path.exists(), "corrupted snapshot must be cleared");
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn clear_all_on_missing_dir_returns_zero() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    assert_eq!(store.clear_all().expect("clear"), 0);
}

#[test]
fn put_overwrites_previous_snapshot() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut state = sample_state("t");
    state.iteration = 1;
    store.put(&state).expect("put 1");
    state.iteration = 7;
    state.updated_at = "2026-05-04T10:05:00Z".into();
    store.put(&state).expect("put 2");

    let loaded = store.get("t").expect("get").expect("present");
    assert_eq!(loaded.iteration, 7);
    assert_eq!(loaded.updated_at, "2026-05-04T10:05:00Z");
}
