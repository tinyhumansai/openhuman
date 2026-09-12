use super::*;

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
        prompt: String::new(),
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
        output: "3 hits".into(),
        arguments: None,
        elapsed_ms: 12,
        iteration: 1,
        failure: None,
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
    // The child tool's (capped) result text is persisted on the call so a
    // rehydrated drawer can show what the tool returned.
    assert_eq!(activity.tool_calls[0].output.as_deref(), Some("3 hits"));
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

/// When a streaming turn is interrupted and a root transcript already exists,
/// `finish()` appends the partial streamed answer (display-only) to the file.
#[test]
fn finish_appends_interrupted_partial_to_existing_transcript() {
    let dir = tempdir().expect("tempdir");
    let thread_id = "thr_abc";
    let path = seed_root_transcript(dir.path(), thread_id);

    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut m = TurnStateMirror::new(store, thread_id, "req-9");
    m.observe(&AgentProgress::IterationStarted {
        iteration: 2,
        max_iterations: 25,
    });
    m.observe(&AgentProgress::ThinkingDelta {
        delta: "hmm".into(),
        iteration: 2,
    });
    m.observe(&AgentProgress::TextDelta {
        delta: "half an ".into(),
        iteration: 2,
    });
    m.observe(&AgentProgress::TextDelta {
        delta: "answer".into(),
        iteration: 2,
    });
    // No TurnCompleted — the bridge exits, marking the turn interrupted.
    m.finish();

    // Model context must NOT carry the partial.
    let model = read_transcript(&path).expect("read model context");
    assert!(
        !model
            .messages
            .iter()
            .any(|msg| msg.content.contains("half an answer")),
        "interrupted partial must be excluded from the model context"
    );

    // Display projection carries the flagged partial with request_id + thinking.
    let display = read_transcript_display(&path).expect("read display");
    let partial = display
        .records
        .iter()
        .find_map(|r| match r {
            DisplayRecord::Message(msg) if msg.interrupted => Some(msg),
            _ => None,
        })
        .expect("display must include the interrupted partial");
    assert_eq!(partial.message.content, "half an answer");
    assert_eq!(partial.request_id.as_deref(), Some("req-9"));
    assert_eq!(partial.iteration, Some(2));
    assert_eq!(partial.reasoning_content.as_deref(), Some("hmm"));
}

/// A completed turn never writes an interrupted partial.
#[test]
fn finish_after_completion_writes_no_partial() {
    let dir = tempdir().expect("tempdir");
    let thread_id = "thr_done";
    let path = seed_root_transcript(dir.path(), thread_id);

    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut m = TurnStateMirror::new(store, thread_id, "req-done");
    m.observe(&AgentProgress::TextDelta {
        delta: "final answer".into(),
        iteration: 1,
    });
    m.observe(&AgentProgress::TurnCompleted { iterations: 1 });
    m.finish();

    let display = read_transcript_display(&path).expect("read display");
    assert!(
        !display
            .records
            .iter()
            .any(|r| matches!(r, DisplayRecord::Message(msg) if msg.interrupted)),
        "a completed turn must not append an interrupted partial"
    );
}

/// An interrupted FIRST turn (no root transcript file yet) is a no-op — the
/// partial stays in the turn_state snapshot only, and finish() does not panic.
#[test]
fn finish_first_turn_without_transcript_is_noop() {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mut m = TurnStateMirror::new(store, "thr_new", "req-first");
    m.observe(&AgentProgress::TextDelta {
        delta: "orphan partial".into(),
        iteration: 1,
    });
    // Must not panic even though no session_raw transcript exists.
    m.finish();
    // The snapshot itself still records the interrupted turn.
    let listed = TurnStateStore::new(dir.path().to_path_buf())
        .get("thr_new")
        .expect("get")
        .expect("snapshot present");
    assert_eq!(listed.lifecycle, TurnLifecycle::Interrupted);
    assert_eq!(listed.streaming_text, "orphan partial");
}

/// A sub-agent's child tool call must persist the arguments it was invoked
/// with. Live, the `subagent_tool_call` socket event carries them; before
/// #5987 the snapshot did not, so a reloaded child row came back with no
/// "Input" block at all.
#[test]
fn subagent_tool_call_persists_its_arguments() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 10,
        prompt: String::new(),
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
    });
    m.observe(&AgentProgress::SubagentToolCallStarted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "tool".into(),
        arguments: serde_json::json!({ "query": "openhuman turn state" }),
        iteration: 1,
        display_label: None,
        display_detail: None,
    });

    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity")
        .clone();
    assert_eq!(
        activity.tool_calls[0].args,
        Some(serde_json::json!({ "query": "openhuman turn state" })),
        "child arguments must reach the snapshot verbatim"
    );

    // The payload is stored exactly once. `output` and `failure` already live
    // only on the call row and are grafted onto the transcript item by
    // `call_id` on the frontend; `args` deliberately follows them rather than
    // duplicating up to 16 KiB into every full-file snapshot rewrite.
    let json = serde_json::to_string(m.snapshot()).expect("serialize");
    assert_eq!(
        json.matches("\"args\"").count(),
        1,
        "arguments must be persisted once, on the call row only"
    );
}

/// A `Value::Null` payload must serialize away rather than persist a
/// meaningless "Input: null" row. This is the state *at start* on the
/// tinyagents path; the arguments arrive later and are backfilled by
/// `tinyagents_path_backfills_arguments_from_the_completion_event`.
#[test]
fn null_child_arguments_are_not_persisted_at_start() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 10,
        prompt: String::new(),
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
    });
    m.observe(&AgentProgress::SubagentToolCallStarted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "search".into(),
        arguments: serde_json::Value::Null,
        iteration: 1,
        display_label: Some("Searching".into()),
        display_detail: None,
    });

    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity")
        .clone();
    assert_eq!(activity.tool_calls[0].args, None);
    let json = serde_json::to_string(m.snapshot()).expect("serialize");
    assert!(
        !json.contains("\"args\""),
        "a null payload must not occupy a field in the snapshot"
    );
}

/// The snapshot file is rewritten in full at every tool boundary, so one
/// `write_file`-shaped payload must not dominate it. An oversized argument
/// blob degrades to a truncated string carrying the same marker shape a
/// truncated tool output uses.
#[test]
fn oversized_child_arguments_are_truncated() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "writer".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 10,
        prompt: String::new(),
        worker_thread_id: None,
        display_name: Some("Writer".into()),
    });
    let huge = "x".repeat(32 * 1024);
    let arguments = serde_json::json!({ "path": "notes.md", "content": huge });
    m.observe(&AgentProgress::SubagentToolCallStarted {
        agent_id: "writer".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "write_file".into(),
        arguments: arguments.clone(),
        iteration: 1,
        display_label: Some("Writing file".into()),
        display_detail: None,
    });

    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity")
        .clone();
    let args = activity.tool_calls[0].args.clone().expect("args persisted");
    let text = args.as_str().expect("oversized args degrade to a string");
    assert!(
        text.len() <= 16 * 1024,
        "truncated arguments must stay within the cap, got {}",
        text.len()
    );
    assert!(
        text.contains("…[truncated"),
        "truncation must be visible to the reader"
    );
    // What is kept is a genuine prefix of the real arguments, not a summary —
    // that leading slice is what tells the reader what the call was doing.
    // Asserted against the serialized head rather than a specific key so the
    // test does not depend on JSON object ordering.
    let kept = text.split('\n').next().expect("prefix line");
    assert!(
        arguments.to_string().starts_with(kept),
        "the persisted text must be the head of the real arguments"
    );
}

/// `args` is additive: a snapshot written before #5987 has no such field and
/// must still deserialize, leaving the row without an input rather than
/// failing the whole thread's rehydration.
#[test]
fn legacy_subagent_tool_call_without_args_deserializes() {
    let legacy = r#"{
        "callId": "c1",
        "toolName": "search",
        "status": "success",
        "iteration": 1,
        "elapsedMs": 12,
        "outputChars": 6,
        "displayName": "Searching",
        "output": "3 hits"
    }"#;
    let call: SubagentToolCall = serde_json::from_str(legacy).expect("legacy row must load");
    assert_eq!(call.args, None);
    assert_eq!(call.call_id, "c1");
    assert_eq!(call.output.as_deref(), Some("3 hits"));
}

/// The ordinary sub-agent tool path — the common case, and the one #5987 was
/// actually reported against. `observability_part_02.rs` emits
/// `SubagentToolCallStarted.arguments` as `Value::Null` there and only supplies
/// the captured input on `SubagentToolCallCompleted.arguments`, so persisting
/// the start event alone leaves these calls with no input after a reload.
#[test]
fn tinyagents_path_backfills_arguments_from_the_completion_event() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 10,
        prompt: String::new(),
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
    });
    // Start carries no arguments — exactly what the tinyagents bridge sends.
    m.observe(&AgentProgress::SubagentToolCallStarted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "web_search".into(),
        arguments: serde_json::Value::Null,
        iteration: 1,
        display_label: Some("Searching the web".into()),
        display_detail: None,
    });
    m.observe(&AgentProgress::SubagentToolCallCompleted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "web_search".into(),
        success: true,
        output_chars: 6,
        output: "3 hits".into(),
        arguments: Some(serde_json::json!({ "query": "latest rust release" })),
        elapsed_ms: 12,
        iteration: 1,
        failure: None,
    });

    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity")
        .clone();
    assert_eq!(
        activity.tool_calls[0].args,
        Some(serde_json::json!({ "query": "latest rust release" })),
        "completion arguments must backfill a call the start event left empty"
    );
}

/// The backfill only fills a gap. A start event that already supplied the
/// arguments stays authoritative, so a completion payload cannot rewrite the
/// input the row was actually invoked with.
#[test]
fn completion_arguments_do_not_overwrite_arguments_captured_at_start() {
    let (_d, mut m) = fresh("t");
    m.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 10,
        prompt: String::new(),
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
    });
    m.observe(&AgentProgress::SubagentToolCallStarted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "web_search".into(),
        arguments: serde_json::json!({ "query": "from start" }),
        iteration: 1,
        display_label: Some("Searching the web".into()),
        display_detail: None,
    });
    m.observe(&AgentProgress::SubagentToolCallCompleted {
        agent_id: "researcher".into(),
        task_id: "sub-1".into(),
        call_id: "c1".into(),
        tool_name: "web_search".into(),
        success: true,
        output_chars: 6,
        output: "3 hits".into(),
        arguments: Some(serde_json::json!({ "query": "from completion" })),
        elapsed_ms: 12,
        iteration: 1,
        failure: None,
    });

    let activity = m.snapshot().tool_timeline[0]
        .subagent
        .as_ref()
        .expect("activity")
        .clone();
    assert_eq!(
        activity.tool_calls[0].args,
        Some(serde_json::json!({ "query": "from start" })),
        "arguments captured at start must not be rewritten on completion"
    );
}
