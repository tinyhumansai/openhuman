//! Unit tests for [`super::TurnStateMirror`].

use super::*;
use crate::openhuman::agent::progress::AgentProgress;
use tempfile::tempdir;

fn fresh(thread_id: &str) -> (tempfile::TempDir, TurnStateMirror) {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mirror = TurnStateMirror::new(store, thread_id, "req-1");
    (dir, mirror)
}

#[test]
fn iteration_start_promotes_lifecycle_and_records_round() {
    let (_d, mut m) = fresh("t");
    let flushed = m.observe(&AgentProgress::IterationStarted {
        iteration: 2,
        max_iterations: 25,
    });
    assert!(flushed);
    let s = m.snapshot();
    assert_eq!(s.lifecycle, TurnLifecycle::Streaming);
    assert_eq!(s.iteration, 2);
    assert_eq!(s.max_iterations, 25);
    assert_eq!(s.phase, Some(TurnPhase::Thinking));
}

#[test]
fn tool_call_start_and_complete_track_timeline() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 25,
    });
    m.observe(&AgentProgress::ToolCallStarted {
        call_id: "tc-1".into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline.len(), 1);
    assert_eq!(s.tool_timeline[0].id, "tc-1");
    assert_eq!(s.tool_timeline[0].status, ToolTimelineStatus::Running);
    assert_eq!(s.active_tool.as_deref(), Some("shell"));

    m.observe(&AgentProgress::ToolCallCompleted {
        call_id: "tc-1".into(),
        tool_name: "shell".into(),
        success: true,
        output_chars: 12,
        elapsed_ms: 50,
        iteration: 1,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline[0].status, ToolTimelineStatus::Success);
    assert!(s.active_tool.is_none());
}

#[test]
fn args_delta_arriving_before_start_creates_placeholder() {
    let (_d, mut m) = fresh("t");
    let flushed = m.observe(&AgentProgress::ToolCallArgsDelta {
        call_id: "tc-9".into(),
        tool_name: "shell".into(),
        delta: "{".into(),
        iteration: 1,
    });
    assert!(!flushed);
    let s = m.snapshot();
    assert_eq!(s.tool_timeline.len(), 1);
    assert_eq!(s.tool_timeline[0].args_buffer.as_deref(), Some("{"));

    m.observe(&AgentProgress::ToolCallArgsDelta {
        call_id: "tc-9".into(),
        tool_name: "shell".into(),
        delta: "\"k\":1}".into(),
        iteration: 1,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline[0].args_buffer.as_deref(), Some("{\"k\":1}"));
}

#[test]
fn tool_call_started_reuses_args_delta_placeholder_for_same_call_id() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 25,
    });
    // Args delta arrives first, before ToolCallStarted.
    m.observe(&AgentProgress::ToolCallArgsDelta {
        call_id: "tc-7".into(),
        tool_name: String::new(),
        delta: "{\"q\":1".into(),
        iteration: 1,
    });
    assert_eq!(m.snapshot().tool_timeline.len(), 1);

    // Start lands — must mutate the placeholder, not append a duplicate.
    m.observe(&AgentProgress::ToolCallStarted {
        call_id: "tc-7".into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
    });
    let timeline = &m.snapshot().tool_timeline;
    assert_eq!(
        timeline.len(),
        1,
        "placeholder must be reused, not duplicated"
    );
    assert_eq!(timeline[0].id, "tc-7");
    assert_eq!(timeline[0].name, "shell");
    assert_eq!(timeline[0].args_buffer.as_deref(), Some("{\"q\":1"));

    // Completion still resolves the same row.
    m.observe(&AgentProgress::ToolCallCompleted {
        call_id: "tc-7".into(),
        tool_name: "shell".into(),
        success: true,
        output_chars: 1,
        elapsed_ms: 5,
        iteration: 1,
    });
    assert_eq!(m.snapshot().tool_timeline.len(), 1);
    assert_eq!(
        m.snapshot().tool_timeline[0].status,
        ToolTimelineStatus::Success
    );
}

#[test]
fn text_delta_appends_streaming_text_without_flushing() {
    let (_d, mut m) = fresh("t");
    assert!(!m.observe(&AgentProgress::TextDelta {
        delta: "hello ".into(),
        iteration: 1,
    }));
    assert!(!m.observe(&AgentProgress::TextDelta {
        delta: "world".into(),
        iteration: 1,
    }));
    assert_eq!(m.snapshot().streaming_text, "hello world");
}

#[test]
fn turn_completed_deletes_snapshot_and_finish_is_noop() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut mirror = TurnStateMirror::new(store.clone(), "t", "req-1");
    mirror.observe(&AgentProgress::TurnCompleted { iterations: 3 });
    assert!(store.get("t").expect("get").is_none());

    // finish() must not resurrect the snapshot.
    mirror.finish();
    assert!(store.get("t").expect("get").is_none());
}

#[test]
fn finish_without_turn_completed_marks_interrupted() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut mirror = TurnStateMirror::new(store.clone(), "t", "req-1");
    mirror.observe(&AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 25,
    });
    mirror.finish();

    let loaded = store.get("t").expect("get").expect("present");
    assert_eq!(loaded.lifecycle, TurnLifecycle::Interrupted);
    assert!(loaded.active_tool.is_none());
}

#[test]
fn subagent_lifecycle_records_and_clears_active() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 25,
    });
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 42,
    });
    let s = m.snapshot();
    assert_eq!(s.active_subagent.as_deref(), Some("researcher"));
    assert_eq!(s.tool_timeline.len(), 1);
    assert_eq!(s.tool_timeline[0].id, "subagent:sub-1");

    m.observe(&AgentProgress::SubagentToolCallStarted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "ctc-1".into(),
        tool_name: "search".into(),
        iteration: 1,
    });
    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity");
    assert_eq!(activity.tool_calls.len(), 1);

    m.observe(&AgentProgress::SubagentCompleted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        elapsed_ms: 1234,
        iterations: 2,
        output_chars: 80,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline[0].status, ToolTimelineStatus::Success);
    assert!(s.active_subagent.is_none());
}
