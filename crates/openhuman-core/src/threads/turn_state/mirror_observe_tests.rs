use super::*;

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
        output: String::new(),
        arguments: None,
        elapsed_ms: 50,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline[0].status, ToolTimelineStatus::Success);
    assert!(s.active_tool.is_none());
    // Empty output (payload capture off) serializes away — no `Some("")`.
    assert!(s.tool_timeline[0].output.is_none());
}

#[test]
fn tool_call_completed_persists_capped_output() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::ToolCallStarted {
        call_id: "tc-1".into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    });
    m.observe(&AgentProgress::ToolCallCompleted {
        call_id: "tc-1".into(),
        tool_name: "shell".into(),
        success: true,
        output_chars: 11,
        output: "hello world".into(),
        arguments: None,
        elapsed_ms: 50,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline[0].output.as_deref(), Some("hello world"));

    // Oversized output is truncated on a char boundary with a marker so the
    // per-flush snapshot rewrite stays bounded.
    m.observe(&AgentProgress::ToolCallStarted {
        call_id: "tc-2".into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 2,
        display_label: None,
        display_detail: None,
    });
    let big = "é".repeat(80 * 1024); // 2 bytes per char > 64 KiB cap
    m.observe(&AgentProgress::ToolCallCompleted {
        call_id: "tc-2".into(),
        tool_name: "shell".into(),
        success: true,
        output_chars: big.chars().count(),
        output: big,
        arguments: None,
        elapsed_ms: 50,
        iteration: 2,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    });
    let s = m.snapshot();
    let persisted = s.tool_timeline[1].output.as_deref().unwrap();
    assert!(persisted.len() <= 64 * 1024);
    assert!(persisted.contains("truncated"));
}

#[test]
fn tool_timeline_entries_carry_monotonic_seq() {
    // Every timeline row is stamped with a per-turn monotonic `seq` at creation
    // so a rehydrated snapshot can order rows identically to the live stream
    // (conversations-timeline-refactor, Phase 4 amendment). Cover the three
    // creation paths: a normal tool start, an args-delta placeholder, and a
    // sub-agent spawn.
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
    // Placeholder-first path (args delta before start) for a second call.
    m.observe(&AgentProgress::ToolCallArgsDelta {
        call_id: "tc-2".into(),
        tool_name: "shell".into(),
        delta: "{".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 10,
        prompt: String::new(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: None,
    });

    let seqs: Vec<u64> = m
        .snapshot()
        .tool_timeline
        .iter()
        .map(|e| e.seq.expect("every row is seq-stamped"))
        .collect();
    assert_eq!(seqs.len(), 3);
    // Strictly increasing in creation order.
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "tool timeline seqs must be strictly increasing, got {seqs:?}"
    );

    // A later `ToolCallStarted` that reuses the args-delta placeholder must NOT
    // restamp the row's seq (the row keeps its original creation order).
    let tc2_seq_before = m
        .snapshot()
        .tool_timeline
        .iter()
        .find(|e| e.id == "tc-2")
        .and_then(|e| e.seq)
        .expect("placeholder seq");
    m.observe(&AgentProgress::ToolCallStarted {
        call_id: "tc-2".into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    });
    let tc2_seq_after = m
        .snapshot()
        .tool_timeline
        .iter()
        .find(|e| e.id == "tc-2")
        .and_then(|e| e.seq)
        .expect("placeholder seq after reuse");
    assert_eq!(
        tc2_seq_before, tc2_seq_after,
        "reusing a placeholder must not restamp its seq"
    );
}

#[test]
fn subagent_prose_item_is_size_capped() {
    // A runaway reasoning stream must not grow a single transcript item without
    // bound (the snapshot is rewritten in full at every flush). Streaming far
    // past the per-item cap coalesces into one item that stays bounded and
    // carries a truncation marker.
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
        prompt: String::new(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: None,
    });
    // 40 KiB of reasoning in same-iteration chunks — must coalesce and cap.
    for _ in 0..40 {
        m.observe(&AgentProgress::SubagentThinkingDelta {
            agent_id: "researcher".into(),
            task_id: "sub-1".into(),
            delta: "x".repeat(1024),
            iteration: 1,
        });
    }
    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity")
        .clone();
    assert_eq!(
        activity.transcript.len(),
        1,
        "same-iteration prose coalesces"
    );
    match &activity.transcript[0] {
        SubagentTranscriptItem::Thinking { text, .. } => {
            assert!(
                text.len() <= super::super::MAX_PERSISTED_TRANSCRIPT_ITEM + 64,
                "capped prose item stays bounded, got {} bytes",
                text.len()
            );
            assert!(text.contains("truncated"), "capped item carries a marker");
        }
        other => panic!("expected thinking, got {other:?}"),
    }
}

#[test]
fn parent_transcript_prose_item_is_size_capped() {
    let (_d, mut m) = fresh("t");
    for _ in 0..40 {
        m.observe(&AgentProgress::ThinkingDelta {
            delta: "y".repeat(1024),
            iteration: 1,
        });
    }
    let s = m.snapshot();
    assert_eq!(s.transcript.len(), 1);
    match &s.transcript[0] {
        TranscriptItem::Thinking { text, .. } => {
            assert!(text.len() <= super::super::MAX_PERSISTED_TRANSCRIPT_ITEM + 64);
            assert!(text.contains("truncated"));
        }
        other => panic!("expected thinking, got {other:?}"),
    }
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
        output: String::new(),
        arguments: None,
        elapsed_ms: 5,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
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
fn turn_completed_keeps_snapshot_as_completed_and_finish_is_noop() {
    let store =
        std::sync::Arc::new(InMemoryTurnStates::default()) as std::sync::Arc<dyn TurnStates>;
    let mut mirror = TurnStateMirror::new(store.clone(), "t", "req-1");
    mirror.observe(&AgentProgress::TurnCompleted {
        iterations: 3,
        stop: None,
    });
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
    let store =
        std::sync::Arc::new(InMemoryTurnStates::default()) as std::sync::Arc<dyn TurnStates>;
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
        prompt: String::new(),
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
        parent_call_id: None,
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
        worktree_path: None,
        changed_files: Vec::new(),
        dirty_status: None,
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        elapsed_ms: 1234,
        iterations: 2,
        output_chars: 80,
        usage: None,
        output: String::new(),
        stop: None,
    });
    let s = m.snapshot();
    assert_eq!(s.tool_timeline[0].status, ToolTimelineStatus::Success);
    assert!(s.active_subagent.is_none());
}

#[path = "mirror_observe_thinking_and_subagent_tests.rs"]
mod thinking_and_subagent_tests;

#[test]
fn thinking_blocks_record_start_and_end_timing() {
    // Regression: the "Thought for Ns" label reads these; a coalesced block
    // must keep its first-delta start and advance its end, and a new round's
    // block must open with its own start.
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::ThinkingDelta {
        delta: "First ".into(),
        iteration: 1,
    });
    let first_start = match &m.snapshot().transcript[0] {
        TranscriptItem::Thinking { started_at, .. } => started_at.expect("started_at stamped"),
        other => panic!("expected thinking, got {other:?}"),
    };
    std::thread::sleep(std::time::Duration::from_millis(5));
    m.observe(&AgentProgress::ThinkingDelta {
        delta: "round.".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::ThinkingDelta {
        delta: "Second round.".into(),
        iteration: 2,
    });

    let s = m.snapshot();
    assert_eq!(s.transcript.len(), 2, "one block per round");
    match &s.transcript[0] {
        TranscriptItem::Thinking {
            started_at,
            ended_at,
            text,
            ..
        } => {
            assert_eq!(text, "First round.");
            assert_eq!(*started_at, Some(first_start), "start is the first delta");
            let end = ended_at.expect("ended_at stamped");
            assert!(end > first_start, "end advances on coalesced deltas");
        }
        other => panic!("expected thinking, got {other:?}"),
    }
    match &s.transcript[1] {
        TranscriptItem::Thinking {
            started_at,
            ended_at,
            ..
        } => {
            let (start, end) = (started_at.unwrap(), ended_at.unwrap());
            assert!(start >= first_start && end >= start);
        }
        other => panic!("expected thinking, got {other:?}"),
    }
}

#[test]
fn thinking_timing_wire_shape_is_camel_case_and_backward_compatible() {
    // The frontend reads `startedAt` / `endedAt`; rows persisted before timing
    // existed must still deserialize (as `None`) and must not grow the fields.
    let item = TranscriptItem::Thinking {
        round: 1,
        seq: 0,
        text: "t".into(),
        started_at: Some(1_000),
        ended_at: Some(13_000),
    };
    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["kind"], "thinking");
    assert_eq!(json["startedAt"], 1_000);
    assert_eq!(json["endedAt"], 13_000);

    let legacy: TranscriptItem =
        serde_json::from_value(serde_json::json!({"kind":"thinking","round":1,"seq":0,"text":"t"}))
            .unwrap();
    match &legacy {
        TranscriptItem::Thinking {
            started_at,
            ended_at,
            ..
        } => assert!(started_at.is_none() && ended_at.is_none()),
        other => panic!("expected thinking, got {other:?}"),
    }
    let reserialized = serde_json::to_value(&legacy).unwrap();
    assert!(reserialized.get("startedAt").is_none());
    assert!(reserialized.get("endedAt").is_none());
}

// ── C1: parent_call_id / source_tool_name derivation ─────────────────────

#[test]
fn subagent_spawned_derives_source_tool_name_from_the_parent_row() {
    // Only `spawn_subagent` ever hardcoded a "spawn_subagent" source. Every
    // other delegation path (`spawn_subagent`,
    // a synthesized `delegate_researcher`, …) must show its own real tool
    // name, derived from the parent call's row by `parent_call_id` — never
    // the historical hardcoded default.
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::ToolCallStarted {
        call_id: "call-parallel".into(),
        tool_name: "spawn_parallel_agents".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    });
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: Some("call-parallel".into()),
    });

    let entry = m
        .snapshot()
        .tool_timeline
        .iter()
        .find(|e| e.id == "subagent:sub-1")
        .cloned()
        .expect("subagent row created");
    assert_eq!(
        entry.source_tool_name.as_deref(),
        Some("spawn_parallel_agents")
    );
    let activity = entry.subagent.expect("subagent activity present");
    assert_eq!(activity.parent_call_id.as_deref(), Some("call-parallel"));
}

#[test]
fn subagent_spawned_falls_back_to_spawn_subagent_without_a_parent_call_id() {
    // No `parent_call_id` (e.g. the `orchestration::ops` spawn path, which
    // has no tool-call context to read one from) keeps the historical
    // default so existing snapshots/consumers don't regress.
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-2".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: None,
    });

    let entry = m
        .snapshot()
        .tool_timeline
        .iter()
        .find(|e| e.id == "subagent:sub-2")
        .cloned()
        .expect("subagent row created");
    assert_eq!(entry.source_tool_name.as_deref(), Some("spawn_subagent"));
    let activity = entry.subagent.expect("subagent activity present");
    assert_eq!(activity.parent_call_id, None);
}

#[test]
fn subagent_completed_persists_capped_output_on_the_activity() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-3".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: Some("call-3".into()),
    });
    m.observe(&AgentProgress::SubagentCompleted {
        worktree_path: None,
        changed_files: Vec::new(),
        dirty_status: None,
        agent_id: "researcher".into(),
        task_id: "sub-3".into(),
        elapsed_ms: 5,
        iterations: 1,
        output_chars: 11,
        usage: None,
        output: "final answer".into(),
        stop: None,
    });

    let entry = m
        .snapshot()
        .tool_timeline
        .iter()
        .find(|e| e.id == "subagent:sub-3")
        .cloned()
        .expect("subagent row created");
    let activity = entry.subagent.expect("subagent activity present");
    assert_eq!(activity.output.as_deref(), Some("final answer"));
    assert_eq!(activity.parent_call_id.as_deref(), Some("call-3"));
}

include!("mirror_observe_compaction_tests.rs");
