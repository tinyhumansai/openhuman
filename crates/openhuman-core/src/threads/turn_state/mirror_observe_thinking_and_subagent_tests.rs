use super::*;

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
    // other delegation path (`spawn_parallel_agents`, `spawn_async_subagent`,
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

#[test]
fn compacting_neither_flushes_nor_changes_the_phase() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 25,
    });
    assert!(!m.observe(&AgentProgress::ContextCompacting));
    assert_eq!(m.snapshot().phase, Some(TurnPhase::Thinking));
}
