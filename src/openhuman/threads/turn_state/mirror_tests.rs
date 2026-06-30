//! Unit tests for [`super::TurnStateMirror`].

use super::*;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::task_board::{TaskBoard, TaskBoardCard, TaskCardStatus};
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
fn transcript_interleaves_narration_thinking_and_tools_in_order() {
    let (_d, mut m) = fresh("t");
    // A turn that thinks, narrates, calls a tool, then narrates again. The
    // transcript must preserve that exact streaming order via `seq`, coalesce
    // consecutive same-kind deltas in the same round, and carry the server
    // label through onto the tool row.
    m.observe(&AgentProgress::ThinkingDelta {
        delta: "Let me ".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::ThinkingDelta {
        delta: "check.".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::TextDelta {
        delta: "Reading your inbox".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::ToolCallStarted {
        call_id: "tc-1".into(),
        tool_name: "gmail_read".into(),
        arguments: serde_json::json!({ "to": "x@y.com" }),
        iteration: 1,
        display_label: Some("Reading messages".into()),
        display_detail: Some("x@y.com".into()),
    });
    m.observe(&AgentProgress::TextDelta {
        delta: "Done.".into(),
        iteration: 1,
    });

    let s = m.snapshot();
    assert_eq!(
        s.transcript.len(),
        4,
        "thinking, narration, tool, narration"
    );
    match &s.transcript[0] {
        TranscriptItem::Thinking { text, seq, .. } => {
            assert_eq!(text, "Let me check.", "coalesced same-round thinking");
            assert_eq!(*seq, 0);
        }
        other => panic!("expected thinking first, got {other:?}"),
    }
    match &s.transcript[1] {
        TranscriptItem::Narration { text, .. } => assert_eq!(text, "Reading your inbox"),
        other => panic!("expected narration, got {other:?}"),
    }
    match &s.transcript[2] {
        TranscriptItem::ToolCall { call_id, .. } => assert_eq!(call_id, "tc-1"),
        other => panic!("expected tool call, got {other:?}"),
    }
    match &s.transcript[3] {
        TranscriptItem::Narration { text, .. } => assert_eq!(text, "Done."),
        other => panic!("expected trailing narration, got {other:?}"),
    }
    // seq is strictly increasing in push order.
    let seqs: Vec<u32> = s
        .transcript
        .iter()
        .map(|i| match i {
            TranscriptItem::Narration { seq, .. }
            | TranscriptItem::Thinking { seq, .. }
            | TranscriptItem::ToolCall { seq, .. } => *seq,
        })
        .collect();
    assert_eq!(seqs, vec![0, 1, 2, 3]);

    // The server label/detail landed on the timeline row the transcript points to.
    let row = s.tool_timeline.iter().find(|e| e.id == "tc-1").unwrap();
    assert_eq!(row.display_name.as_deref(), Some("Reading messages"));
    assert_eq!(row.detail.as_deref(), Some("x@y.com"));
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
        display_label: None,
        display_detail: None,
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
        display_label: None,
        display_detail: None,
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
fn task_board_update_is_stored_and_flushed() {
    let (dir, mut m) = fresh("t");
    let board = TaskBoard {
        thread_id: "t".into(),
        cards: vec![TaskBoardCard {
            id: "task-1".into(),
            title: "Draft".into(),
            status: TaskCardStatus::Todo,
            objective: None,
            plan: Vec::new(),
            assigned_agent: None,
            allowed_tools: Vec::new(),
            approval_mode: None,
            acceptance_criteria: Vec::new(),
            evidence: Vec::new(),
            notes: None,
            blocker: None,
            session_thread_id: None,
            source_metadata: None,
            order: 0,
            updated_at: "2026-05-15T00:00:00Z".into(),
        }],
        updated_at: "2026-05-15T00:00:00Z".into(),
    };
    assert!(m.observe(&AgentProgress::TaskBoardUpdated {
        board: board.clone()
    }));
    assert_eq!(m.snapshot().task_board.as_ref(), Some(&board));

    let loaded = TurnStateStore::new(dir.path().to_path_buf())
        .get("t")
        .expect("load flushed snapshot")
        .expect("snapshot exists");
    assert_eq!(loaded.task_board, Some(board));
}

#[test]
fn turn_completed_keeps_snapshot_as_completed_and_finish_is_noop() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut mirror = TurnStateMirror::new(store.clone(), "t", "req-1");
    mirror.observe(&AgentProgress::TurnCompleted { iterations: 3 });
    // The snapshot is kept (not deleted) so a reloaded client can replay the
    // finished turn's processing transcript, marked terminal `Completed` with
    // the live fields quiesced.
    let loaded = store.get("t").expect("get").expect("snapshot kept");
    assert_eq!(loaded.lifecycle, TurnLifecycle::Completed);
    assert!(loaded.active_tool.is_none());
    assert!(loaded.active_subagent.is_none());
    assert!(loaded.phase.is_none());

    // finish() must not flip a completed snapshot back to interrupted.
    mirror.finish();
    let after = store.get("t").expect("get").expect("snapshot still kept");
    assert_eq!(after.lifecycle, TurnLifecycle::Completed);
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
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
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
        arguments: serde_json::Value::Null,
        iteration: 1,
        display_label: None,
        display_detail: None,
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
        worktree_path: None,
        changed_files: Vec::new(),
        dirty_status: None,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline[0].status, ToolTimelineStatus::Success);
    assert!(s.active_subagent.is_none());
}

#[test]
fn subagent_transcript_persists_interleaved_prose_and_tools() {
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
        prompt_chars: 10,
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
    });
    // Reasoning (two same-iteration deltas, must coalesce), then a tool, then
    // visible narration — the order must be preserved in the transcript.
    m.observe(&AgentProgress::SubagentThinkingDelta {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        delta: "let me ".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::SubagentThinkingDelta {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        delta: "search.".into(),
        iteration: 1,
    });
    // A sub-agent tool boundary must flush the accumulated prose to disk.
    let flushed = m.observe(&AgentProgress::SubagentToolCallStarted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "search".into(),
        arguments: serde_json::Value::Null,
        iteration: 1,
        display_label: Some("Searching".into()),
        display_detail: None,
    });
    assert!(flushed, "sub-agent tool boundary must flush");
    m.observe(&AgentProgress::SubagentTextDelta {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        delta: "Found it.".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::SubagentToolCallCompleted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "search".into(),
        success: true,
        output_chars: 5,
        output: String::new(),
        elapsed_ms: 12,
        iteration: 1,
    });

    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity")
        .clone();
    assert_eq!(activity.transcript.len(), 3, "thinking, tool, narration");
    match &activity.transcript[0] {
        SubagentTranscriptItem::Thinking { text, .. } => {
            assert_eq!(text, "let me search.", "coalesced same-iteration thinking");
        }
        other => panic!("expected thinking, got {other:?}"),
    }
    match &activity.transcript[1] {
        SubagentTranscriptItem::Tool {
            call_id, status, ..
        } => {
            assert_eq!(call_id, "c1");
            // Completion flips the transcript tool item, not just `tool_calls`.
            assert_eq!(*status, ToolTimelineStatus::Success);
        }
        other => panic!("expected tool, got {other:?}"),
    }
    match &activity.transcript[2] {
        SubagentTranscriptItem::Text { text, .. } => assert_eq!(text, "Found it."),
        other => panic!("expected narration, got {other:?}"),
    }

    // The wire form MUST be camelCase — the FE reads `toolName`/`callId`, and
    // snake_case leaking through caused a `replace`-on-undefined crash.
    let json = serde_json::to_string(m.snapshot()).expect("serialize");
    assert!(
        json.contains("\"toolName\""),
        "tool item must serialize camelCase"
    );
    assert!(json.contains("\"callId\""));
    assert!(
        !json.contains("\"tool_name\""),
        "no snake_case fields on the wire"
    );
}
