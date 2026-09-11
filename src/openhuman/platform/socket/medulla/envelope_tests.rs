use super::*;

#[test]
fn maps_text_delta_to_agent_message() {
    let p = AgentProgress::TextDelta {
        delta: "hello".to_string(),
        iteration: 1,
    };
    let kind = progress_to_event_kind(&p).expect("mapped");
    assert!(matches!(kind, HarnessEventKind::AgentMessage(ref t) if t.text == "hello"));
}

#[test]
fn maps_thinking_delta_to_agent_thinking() {
    let p = AgentProgress::ThinkingDelta {
        delta: "hmm".to_string(),
        iteration: 1,
    };
    assert!(matches!(
        progress_to_event_kind(&p),
        Some(HarnessEventKind::AgentThinking(_))
    ));
}

#[test]
fn drops_non_stream_variants() {
    let p = AgentProgress::TurnContent {
        input: Some("q".to_string()),
        output: Some("a".to_string()),
    };
    assert!(progress_to_event_kind(&p).is_none());
}

#[test]
fn envelope_is_valid_and_round_trips_through_the_wire() {
    let kind = HarnessEventKind::AgentMessage(TextPayload {
        text: "done".to_string(),
    });
    let env = envelope_for_kind("sess-1", 7, &kind);
    assert!(env.is_valid());
    assert_eq!(env.envelope_version, "tinyplace.harness.session.v2");
    assert_eq!(env.version, 2);
    assert_eq!(env.event.kind, "agent_message");
    assert_eq!(env.event.seq, 7);

    // Serialize and parse back through the Medulla decoder.
    let wire_value = serde_json::to_value(&env).unwrap();
    assert_eq!(
        wire_value["bucket"],
        serde_json::json!({"unit":"","start":"","end":""})
    );
    assert_eq!(
        wire_value["harness"],
        serde_json::json!({"provider":"","command":"","argv":[]})
    );
    assert_eq!(
        wire_value["source"],
        serde_json::json!({"path":"","record_type":""})
    );
    assert_eq!(wire_value["scope"]["type"], "session");
    assert_eq!(wire_value["event"]["kind"], "agent_message");
    let wrapper = super::super::payloads::TaskEnvelope {
        task_id: "task-1".to_string(),
        envelope: wire_value.clone(),
    };
    let wrapper_wire = serde_json::to_value(wrapper).unwrap();
    assert_eq!(wrapper_wire["taskId"], "task-1");
    assert_eq!(wrapper_wire["envelope"], wire_value);

    let wire = serde_json::to_string(&env).unwrap();
    let parsed = HarnessEnvelope::parse(&wire).expect("valid Medulla wire");
    match parsed.event.decoded() {
        HarnessEventKind::AgentMessage(t) => assert_eq!(t.text, "done"),
        other => panic!("unexpected decoded kind: {other:?}"),
    }
}

#[test]
fn established_event_tags_and_unknown_fallback_are_preserved() {
    let user = serde_json::to_value(HarnessEventKind::UserPrompt(UserPromptPayload {
        text: "hello".to_string(),
        source: "human".to_string(),
    }))
    .unwrap();
    assert_eq!(user["kind"], "user_prompt");
    assert_eq!(user["payload"]["source"], "human");

    let lifecycle = serde_json::to_value(HarnessEventKind::Lifecycle(LifecyclePayload {
        phase: "turn_end".to_string(),
    }))
    .unwrap();
    assert_eq!(lifecycle["kind"], "lifecycle");

    let raw = serde_json::json!({"future": true});
    let event = HarnessEvent {
        kind: "future_kind".to_string(),
        payload: raw.clone(),
        ..Default::default()
    };
    assert!(matches!(
        event.decoded(),
        HarnessEventKind::Unknown(UnknownPayload { raw: value }) if value == raw
    ));
}

#[test]
fn tool_call_and_result_map_to_their_kinds() {
    let started = AgentProgress::ToolCallStarted {
        call_id: "c1".to_string(),
        tool_name: "Bash".to_string(),
        arguments: serde_json::json!({ "cmd": "ls" }),
        iteration: 1,
        display_label: Some("List files".to_string()),
        display_detail: None,
    };
    match progress_to_event_kind(&started) {
        Some(HarnessEventKind::ToolCall(tc)) => {
            assert_eq!(tc.call_id, "c1");
            assert_eq!(tc.tool_name, "Bash");
            assert_eq!(tc.display, "List files");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }

    let completed = AgentProgress::ToolCallCompleted {
        call_id: "c1".to_string(),
        tool_name: "Bash".to_string(),
        success: true,
        output_chars: 3,
        output: "ok\n".to_string(),
        arguments: None,
        elapsed_ms: 5,
        iteration: 1,
        failure: None,
    };
    match progress_to_event_kind(&completed) {
        Some(HarnessEventKind::ToolResult(tr)) => {
            assert!(tr.ok);
            assert!(!tr.is_error);
            assert_eq!(tr.output, "ok\n");
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}
