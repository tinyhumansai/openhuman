//! Unit tests for [`super::TurnStateStore`].

use super::*;
use crate::openhuman::threads::turn_state::types::{
    SubagentActivity, SubagentToolCall, SubagentTranscriptItem, ToolTimelineEntry,
    ToolTimelineStatus, TurnLifecycle, TurnState,
};
use tempfile::tempdir;

fn sample_state(thread_id: &str) -> TurnState {
    TurnState::started(thread_id.to_string(), "req-1", 25, "2026-05-04T10:00:00Z")
}

/// A turn with an explicit request id + started/updated timestamps.
fn turn(thread_id: &str, request_id: &str, started_at: &str) -> TurnState {
    let mut s = TurnState::started(thread_id.to_string(), request_id, 25, started_at);
    s.updated_at = started_at.to_string();
    s
}

fn turn_states_root(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path()
        .join("memory")
        .join("conversations")
        .join("turn_states")
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
        failure: None,
        output: None,
        seq: Some(0),
    });

    store.put(&state).expect("put");
    let loaded = store.get("thread-abc").expect("get").expect("present");
    assert_eq!(loaded, state);
}

#[test]
fn roundtrips_subagent_interleaved_transcript_with_full_fidelity() {
    // A settled turn whose subagent streamed reasoning, called a tool, then
    // narrated — the interleaved transcript (not just the flat tool rows) plus
    // the per-row `seq` ordering keys must survive a disk round-trip verbatim,
    // so a reopened transcript rehydrates without losing the subagent's
    // reasoning (the gap SubagentDrawer/chatRuntimeSlice documented).
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut state = sample_state("thread-sub");
    state.lifecycle = TurnLifecycle::Completed;

    let activity = SubagentActivity {
        task_id: "sub-1".into(),
        agent_id: "researcher".into(),
        status: Some("completed".into()),
        mode: Some("typed".into()),
        dedicated_thread: Some(false),
        child_iteration: Some(1),
        child_max_iterations: Some(8),
        iterations: Some(2),
        elapsed_ms: Some(1234),
        output_chars: Some(42),
        worker_thread_id: Some("worker-thread-9".into()),
        tool_calls: vec![SubagentToolCall {
            call_id: "c1".into(),
            tool_name: "search".into(),
            status: ToolTimelineStatus::Success,
            iteration: Some(1),
            elapsed_ms: Some(12),
            output_chars: Some(6),
            display_name: Some("Searching".into()),
            detail: None,
            failure: None,
            output: Some("3 hits".into()),
        }],
        transcript: vec![
            SubagentTranscriptItem::Thinking {
                iteration: Some(1),
                text: "let me search.".into(),
            },
            SubagentTranscriptItem::Tool {
                iteration: Some(1),
                call_id: "c1".into(),
                tool_name: "search".into(),
                status: ToolTimelineStatus::Success,
                elapsed_ms: Some(12),
                output_chars: Some(6),
                display_name: Some("Searching".into()),
                detail: None,
            },
            SubagentTranscriptItem::Text {
                iteration: Some(1),
                text: "Found it.".into(),
            },
        ],
    };

    state.tool_timeline.push(ToolTimelineEntry {
        id: "subagent:sub-1".into(),
        name: "subagent:researcher".into(),
        round: 1,
        status: ToolTimelineStatus::Success,
        args_buffer: None,
        display_name: Some("Researcher".into()),
        detail: None,
        source_tool_name: Some("spawn_subagent".into()),
        subagent: Some(activity),
        failure: None,
        output: None,
        seq: Some(3),
    });

    store.put(&state).expect("put");
    let loaded = store.get("thread-sub").expect("get").expect("present");
    // Structural equality proves nothing in the interleaved transcript,
    // subagent activity, or the `seq` ordering keys was dropped or reordered.
    assert_eq!(loaded, state);

    // Spot-check the interleaving explicitly so a future regression that keeps
    // the fields but loses the ordering still fails here.
    let activity = loaded.tool_timeline[0]
        .subagent
        .as_ref()
        .expect("subagent activity restored");
    assert_eq!(activity.transcript.len(), 3);
    assert!(matches!(
        activity.transcript[0],
        SubagentTranscriptItem::Thinking { .. }
    ));
    assert!(matches!(
        activity.transcript[1],
        SubagentTranscriptItem::Tool { .. }
    ));
    assert!(matches!(
        activity.transcript[2],
        SubagentTranscriptItem::Text { .. }
    ));
    assert_eq!(loaded.tool_timeline[0].seq, Some(3));
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
fn mark_all_interrupted_leaves_completed_snapshots_untouched() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut state = sample_state("t");
    // A finished turn is kept as `Completed` so its processing can be replayed;
    // startup interrupted-marking must not flip it to `Interrupted`.
    state.lifecycle = TurnLifecycle::Completed;
    store.put(&state).expect("put");

    let count = store
        .mark_all_interrupted("2026-05-04T10:01:00Z")
        .expect("mark");
    assert_eq!(count, 0);
    let loaded = store.get("t").expect("get").expect("present");
    assert_eq!(loaded.lifecycle, TurnLifecycle::Completed);
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
fn keeps_a_separate_snapshot_per_turn_and_get_returns_latest() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    store
        .put(&turn("t", "req-1", "2026-05-04T10:00:00Z"))
        .expect("put turn 1");
    store
        .put(&turn("t", "req-2", "2026-05-04T10:05:00Z"))
        .expect("put turn 2");

    // Both turns are retained...
    let history = store.list_thread("t").expect("list_thread");
    assert_eq!(history.len(), 2);
    // ...newest first.
    assert_eq!(history[0].request_id, "req-2");
    assert_eq!(history[1].request_id, "req-1");

    // get(thread) resolves the latest turn (greatest started_at).
    let latest = store.get("t").expect("get").expect("present");
    assert_eq!(latest.request_id, "req-2");

    // get_turn fetches a specific earlier turn.
    let earlier = store
        .get_turn("t", "req-1")
        .expect("get_turn")
        .expect("present");
    assert_eq!(earlier.request_id, "req-1");
    assert!(store.get_turn("t", "nope").expect("get_turn").is_none());

    // list() surfaces exactly one (latest) entry per thread for cold boot.
    let latest_per_thread = store.list().expect("list");
    assert_eq!(latest_per_thread.len(), 1);
    assert_eq!(latest_per_thread[0].request_id, "req-2");
}

#[test]
fn completed_turns_are_pruned_to_the_retention_window() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    // Write 25 completed turns; only the newest COMPLETED_RETENTION (20) survive.
    for i in 0..25 {
        let mut s = turn(
            "t",
            &format!("req-{i:02}"),
            &format!("2026-05-04T10:{i:02}:00Z"),
        );
        s.lifecycle = TurnLifecycle::Completed;
        s.updated_at = format!("2026-05-04T10:{i:02}:00Z");
        store.put(&s).expect("put");
    }
    let history = store.list_thread("t").expect("list_thread");
    assert_eq!(history.len(), super::COMPLETED_RETENTION);
    // The oldest five (req-00..req-04) are gone; the newest survive.
    assert!(history.iter().any(|t| t.request_id == "req-24"));
    assert!(history.iter().all(|t| t.request_id != "req-00"));
    assert!(store.get_turn("t", "req-04").expect("get_turn").is_none());
    assert!(store.get_turn("t", "req-05").expect("get_turn").is_some());
}

#[test]
fn a_live_turn_is_not_pruned_alongside_completed_history() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    for i in 0..COMPLETED_RETENTION_PLUS {
        let mut s = turn(
            "t",
            &format!("done-{i:02}"),
            &format!("2026-05-04T10:{i:02}:00Z"),
        );
        s.lifecycle = TurnLifecycle::Completed;
        s.updated_at = format!("2026-05-04T10:{i:02}:00Z");
        store.put(&s).expect("put completed");
    }
    // A running turn coexists and is never pruned (only completed turns are).
    let mut live = turn("t", "live", "2026-05-04T11:00:00Z");
    live.lifecycle = TurnLifecycle::Streaming;
    store.put(&live).expect("put live");
    assert!(store.get_turn("t", "live").expect("get_turn").is_some());
    assert_eq!(
        store.get("t").expect("get").expect("present").request_id,
        "live"
    );
}

#[test]
fn migrates_a_legacy_flat_snapshot_into_the_per_turn_layout() {
    use std::io::Write as _;
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let root = turn_states_root(&dir);
    std::fs::create_dir_all(&root).expect("mkdir root");

    // Hand-write an old-style flat snapshot at `<hex(thread_id)>.json`.
    let legacy = turn("legacy-thread", "req-legacy", "2026-05-04T09:00:00Z");
    let flat_path = root.join(format!("{}.json", hex::encode("legacy-thread".as_bytes())));
    let mut f = std::fs::File::create(&flat_path).expect("create flat");
    f.write_all(serde_json::to_vec_pretty(&legacy).unwrap().as_slice())
        .expect("write flat");
    drop(f);

    // First access migrates it into the dir and removes the flat file.
    let loaded = store.get("legacy-thread").expect("get").expect("present");
    assert_eq!(loaded.request_id, "req-legacy");
    assert!(
        !flat_path.exists(),
        "flat file must be removed after migration"
    );
    let per_turn = root
        .join(hex::encode("legacy-thread".as_bytes()))
        .join(format!("{}.json", hex::encode("req-legacy".as_bytes())));
    assert!(
        per_turn.exists(),
        "snapshot must live under the per-turn path"
    );

    // Migration is idempotent — a second access is a no-op.
    assert_eq!(
        store
            .get("legacy-thread")
            .expect("get2")
            .expect("present")
            .request_id,
        "req-legacy"
    );
}

const COMPLETED_RETENTION_PLUS: usize = super::COMPLETED_RETENTION + 3;

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
