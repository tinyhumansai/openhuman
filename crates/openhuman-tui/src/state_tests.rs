use super::*;

#[test]
fn viewport_journal_is_bounded_and_identifies_non_tail_changes_and_resets() {
    let mut state = TranscriptState::new("client");
    state.push_system("first");
    state.push_system("second");
    let snapshot = state.viewport_revision();
    assert_eq!(state.viewport_changes_since(snapshot), Some(vec![]));
    state.toggle_entry(0);
    state.push_system("third");
    assert_eq!(state.viewport_changes_since(snapshot), Some(vec![0, 2]));
    for _ in 0..140 {
        state.push_system("more");
    }
    assert_eq!(state.viewport_changes.len(), 128);
    assert_eq!(state.viewport_changes_since(snapshot), None);
    let snapshot = state.viewport_revision();
    state.load_transcript(&serde_json::json!({"items":[]}));
    assert_eq!(state.viewport_changes_since(snapshot), None);
    let snapshot = state.viewport_revision();
    state.clear();
    assert_eq!(state.viewport_changes_since(snapshot), None);
}

const CLIENT: &str = "tui-abc123";

fn ev(event: &str, fields: serde_json::Value) -> WebChannelEvent {
    let mut value = serde_json::json!({
        "event": event,
        "client_id": CLIENT,
        "thread_id": "thread-1",
        "request_id": "request-1"
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    serde_json::from_value(value).expect("test event matches the owner contract")
}

fn text_delta(delta: &str) -> WebChannelEvent {
    ev(
        "text_delta",
        serde_json::json!({"delta":delta,"delta_kind":"text"}),
    )
}

fn thinking_delta(delta: &str) -> WebChannelEvent {
    ev(
        "thinking_delta",
        serde_json::json!({"delta":delta,"delta_kind":"thinking"}),
    )
}

#[test]
fn text_deltas_accumulate_into_one_assistant_entry() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("hi");
    s.apply_event(&text_delta("Hel"));
    s.apply_event(&text_delta("lo "));
    s.apply_event(&text_delta("world"));

    let assistant: Vec<_> = s
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Assistant)
        .collect();
    assert_eq!(assistant.len(), 1, "deltas must fold into a single entry");
    assert_eq!(assistant[0].text, "Hello world");
    assert!(s.is_streaming(), "still streaming before chat_done");
}

#[test]
fn thinking_and_text_are_separate_entries() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("q");
    s.apply_event(&thinking_delta("let me think"));
    s.apply_event(&text_delta("answer"));

    let kinds: Vec<_> = s.entries().iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![EntryKind::User, EntryKind::Thinking, EntryKind::Assistant]
    );
    let thinking = s
        .entries()
        .iter()
        .find(|e| e.kind == EntryKind::Thinking)
        .unwrap();
    assert_eq!(thinking.text, "let me think");
}

#[test]
fn thinking_deltas_accumulate_separately_from_text() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("q");
    s.apply_event(&thinking_delta("a"));
    s.apply_event(&thinking_delta("b"));
    s.apply_event(&text_delta("x"));
    s.apply_event(&thinking_delta("c")); // interleaved — same thinking entry
    let thinking: Vec<_> = s
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Thinking)
        .collect();
    assert_eq!(thinking.len(), 1);
    assert_eq!(thinking[0].text, "abc");
}

#[test]
fn chat_done_replaces_streamed_text_with_full_response() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("hi");
    s.apply_event(&text_delta("Hel")); // partial / laggy stream
    let done = ev(
        "chat_done",
        serde_json::json!({"full_response":"Hello, world!"}),
    );
    s.apply_event(&done);

    let assistant: Vec<_> = s
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Assistant)
        .collect();
    assert_eq!(assistant.len(), 1);
    assert_eq!(
        assistant[0].text, "Hello, world!",
        "full_response is authoritative and replaces the streamed text"
    );
    assert!(!s.is_streaming(), "chat_done ends the turn");
}

#[test]
fn chat_done_without_prior_deltas_still_shows_full_response() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("hi");
    let done = ev(
        "chat_done",
        serde_json::json!({"full_response":"Direct answer"}),
    );
    s.apply_event(&done);
    let assistant = s
        .entries()
        .iter()
        .find(|e| e.kind == EntryKind::Assistant)
        .expect("chat_done with full_response must produce an assistant entry");
    assert_eq!(assistant.text, "Direct answer");
    assert!(!s.is_streaming());
}

#[test]
fn channel_outbound_finishes_turn_with_its_final_response() {
    let mut state = TranscriptState::new(CLIENT);
    state.begin_user_turn("hi");
    state.apply_event(&text_delta("partial"));
    state.apply_event(&ev(
        "channel_outbound",
        serde_json::json!({"full_response":"final reply"}),
    ));
    let assistant = state
        .entries()
        .iter()
        .find(|entry| entry.kind == EntryKind::Assistant)
        .expect("channel_outbound carries the delivered assistant reply");
    assert_eq!(assistant.text, "final reply");
    assert!(!state.is_streaming());
}

#[test]
fn chat_error_pushes_error_entry_and_ends_stream() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("hi");
    let err = ev(
        "chat_error",
        serde_json::json!({"message":"rate limited","error_type":"rate_limit"}),
    );
    s.apply_event(&err);
    let error = s
        .entries()
        .iter()
        .find(|e| e.kind == EntryKind::Error)
        .expect("chat_error must produce an error entry");
    assert_eq!(error.text, "rate limited");
    assert!(!s.is_streaming(), "chat_error ends the turn");
}

#[test]
fn retryable_error_includes_retry_timing() {
    let mut state = TranscriptState::new(CLIENT);
    state.begin_user_turn("try");
    state.apply_event(&ev(
        "chat_error",
        serde_json::json!({
            "message":"rate limited","error_retryable":true,"error_retry_after_ms":2100
        }),
    ));
    assert_eq!(
        state.entries().last().unwrap().text,
        "rate limited · retryable after 3s"
    );
}

#[test]
fn last_assistant_excludes_an_in_flight_partial_answer() {
    let mut state = TranscriptState::new(CLIENT);
    state.begin_user_turn("question");
    state.apply_event(&text_delta("partial"));
    assert_eq!(state.last_assistant(), None);
    state.apply_event(&ev(
        "chat_done",
        serde_json::json!({"full_response":"complete"}),
    ));
    assert_eq!(state.last_assistant(), Some("complete"));
}

#[test]
fn events_for_other_client_id_are_ignored() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("hi");
    let before = s.entries().len();
    let foreign = {
        let mut event =
            serde_json::to_value(ev("text_delta", serde_json::json!({"delta":"not mine"})))
                .unwrap();
        event["client_id"] = "tui-someone-else".into();
        serde_json::from_value(event).unwrap()
    };
    s.apply_event(&foreign);
    assert_eq!(
        s.entries().len(),
        before,
        "foreign client_id events must not mutate our transcript"
    );
}

#[test]
fn events_for_an_inactive_thread_are_ignored() {
    let mut state = TranscriptState::new(CLIENT);
    state.set_thread("thread-1");
    let event = {
        let mut event = serde_json::to_value(text_delta("not this conversation")).unwrap();
        event["thread_id"] = "thread-2".into();
        serde_json::from_value(event).unwrap()
    };
    state.apply_event(&event);
    assert!(state.entries().is_empty());
}

#[test]
fn projected_transcript_is_restored_in_chronological_order() {
    let mut state = TranscriptState::new(CLIENT);
    state.load_transcript(&serde_json::json!({
        "result": {"data": {"items": [
            {"kind": "assistantMessage", "content": "answer"},
            {"kind": "userMessage", "content": "question"}
        ]}}
    }));
    assert_eq!(state.entries()[0].text, "question");
    assert_eq!(state.entries()[1].text, "answer");
}

#[test]
fn projected_voice_note_keeps_its_transcript_and_duration_label() {
    let mut state = TranscriptState::new(CLIENT);
    state.load_transcript(&serde_json::json!({
        "result": {"data": {"items": [
            {
                "kind": "userMessage",
                "content": "Remind me about the dentist",
                "extraMetadata": {"voiceNote": {"noteId": "recording-1", "durationMs": 12000}}
            }
        ]}}
    }));
    assert_eq!(
        state.entries()[0].text,
        "[voice note 0:12] Remind me about the dentist"
    );
}

#[test]
fn projected_spoken_replies_show_voice_note_and_known_duration_labels() {
    let mut state = TranscriptState::new(CLIENT);
    state.load_transcript(&serde_json::json!({
        "result": {"data": {"items": [
            {
                "kind": "assistantMessage",
                "content": "I can say that aloud.",
                "extraMetadata": {"voiceNote": {"noteId": "reply-2", "mime": "audio/wav", "visemes": [{"viseme": "aa"}]}}
            },
            {
                "kind": "assistantMessage",
                "content": "I will remind you.",
                "extraMetadata": {"voiceNote": {"noteId": "reply-1", "mime": "audio/wav", "durationMs": 8600}}
            }
        ]}}
    }));
    assert_eq!(
        state.entries()[0].text,
        "[voice note 0:09] I will remind you."
    );
    assert_eq!(
        state.entries()[1].text,
        "[voice note] I can say that aloud."
    );
}

#[test]
fn tool_result_updates_the_originating_activity() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("do it");
    let call = ev(
        "tool_call",
        serde_json::json!({
            "tool_name":"web_search","args":{"query":"rust ratatui"}
        }),
    );
    s.apply_event(&call);
    let result = ev(
        "tool_result",
        serde_json::json!({
            "tool_name":"web_search","success":true,"output":"3 results"
        }),
    );
    s.apply_event(&result);

    let tools: Vec<_> = s
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Tool)
        .collect();
    assert_eq!(tools.len(), 1);
    assert!(tools[0].text.starts_with("✓ web_search"));
    let activity = tools[0].activity.as_ref().unwrap();
    assert!(activity.args.contains("rust ratatui"));
    assert_eq!(activity.output, "3 results");
}

#[test]
fn failed_tool_result_uses_cross_marker() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("do it");
    let result = ev(
        "tool_result",
        serde_json::json!({
            "tool_name":"run_shell","success":false,"output":"exit code 1"
        }),
    );
    s.apply_event(&result);
    let tool = s
        .entries()
        .iter()
        .find(|e| e.kind == EntryKind::Tool)
        .unwrap();
    assert!(tool.text.starts_with("✗ run_shell"));
}

#[test]
fn second_turn_opens_fresh_assistant_entry() {
    let mut s = TranscriptState::new(CLIENT);
    s.begin_user_turn("first");
    s.apply_event(&text_delta("one"));
    s.apply_event(&ev("chat_done", serde_json::json!({"full_response":"one"})));
    s.begin_user_turn("second");
    s.apply_event(&text_delta("two"));

    let assistant: Vec<_> = s
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Assistant)
        .collect();
    assert_eq!(assistant.len(), 2, "each turn gets its own assistant entry");
    assert_eq!(assistant[0].text, "one");
    assert_eq!(assistant[1].text, "two");
}

#[test]
fn truncate_line_collapses_newlines_and_caps_length() {
    let long = "a\nb\n".repeat(200);
    let out = truncate_line(&long);
    assert!(!out.contains('\n'));
    assert!(out.chars().count() <= 121, "capped to MAX + ellipsis");
}

#[test]
fn concurrent_same_named_tools_match_results_by_call_identity() {
    let mut state = TranscriptState::new(CLIENT);
    state.set_thread("thread-1");
    for id in ["a", "b"] {
        state.apply_event(&ev(
            "tool_call",
            serde_json::json!({
                "tool_call_id":id,"tool_name":"read","request_id":"turn"
            }),
        ));
    }
    state.apply_event(&ev(
        "tool_result",
        serde_json::json!({
            "tool_call_id":"b","tool_name":"read","request_id":"turn",
            "success":false,"output":"failure"
        }),
    ));
    assert_eq!(
        state.entries()[0].activity.as_ref().unwrap().status,
        Status::Running
    );
    assert_eq!(
        state.entries()[1].activity.as_ref().unwrap().status,
        Status::Error
    );
    state.apply_event(&ev(
        "tool_result",
        serde_json::json!({
            "tool_call_id":"b","request_id":"turn","success":false
        }),
    ));
    assert_eq!(state.entries().len(), 2);
}

#[test]
fn child_activity_accumulates_output_and_failed_status_in_one_record() {
    let mut state = TranscriptState::new(CLIENT);
    for (event, delta) in [
        ("subagent_spawned", None),
        ("subagent_text_delta", Some("partial")),
        ("subagent_failed", None),
    ] {
        state.apply_event(&ev(
            event,
            serde_json::json!({
                "skill_id":"task","request_id":"turn","delta":delta,"message":"Timed out"
            }),
        ));
    }
    assert_eq!(state.entries().len(), 1);
    let child = state.entries()[0].activity.as_ref().unwrap();
    assert!(child.child);
    assert_eq!(child.status, Status::Error);
    assert_eq!(child.output, "partial");
    assert!(child.details().contains("Timed out"));
}

#[test]
fn child_thinking_delta_uses_nested_task_identity() {
    let mut state = TranscriptState::new(CLIENT);
    for (event, delta) in [
        ("subagent_spawned", None),
        ("subagent_text_delta", Some("partial")),
        ("subagent_thinking_delta", Some("reasoning")),
        ("subagent_failed", None),
    ] {
        state.apply_event(&ev(
            event,
            serde_json::json!({
                "skill_id":null,"request_id":"turn","delta":delta,"message":"Timed out",
                "subagent":{"task_id":"task"}
            }),
        ));
    }
    assert_eq!(state.entries().len(), 1);
    let child = state.entries()[0].activity.as_ref().unwrap();
    assert!(child.child);
    assert_eq!(child.status, Status::Error);
    assert_eq!(child.output, "partial");
}

#[test]
fn sequence_duplicates_do_not_duplicate_streamed_text() {
    let mut state = TranscriptState::new(CLIENT);
    let event = ev(
        "text_delta",
        serde_json::json!({"request_id":"turn","seq":0,"delta":"one","delta_kind":"text"}),
    );
    state.apply_event(&event);
    state.apply_event(&event);
    assert_eq!(state.entries()[0].text, "one");
}

#[test]
fn replay_restores_inspectable_tool_and_child_output_without_flattening() {
    let mut state = TranscriptState::new(CLIENT);
    state.load_transcript(&serde_json::json!({"items":[{"kind":"subagent","id":"child","agentId":"Explore","status":"completed","items":[{"kind":"assistantMessage","content":"child answer"}]},{"kind":"toolCall","callId":"a","name":"read","args":{"file":"a.rs"},"result":"file body","status":"success"}]}));
    assert_eq!(state.entries().len(), 2);
    assert_eq!(
        state.entries()[0].activity.as_ref().unwrap().output,
        "file body"
    );
    assert_eq!(
        state.entries()[1].activity.as_ref().unwrap().output,
        "child answer"
    );
}

#[test]
fn clearing_display_during_streaming_keeps_turn_and_resets_cursors() {
    let mut state = TranscriptState::new(CLIENT);
    state.begin_user_turn("question");
    state.apply_event(&text_delta("first"));
    state.clear();
    state.apply_event(&text_delta("next"));
    assert!(state.is_streaming());
    assert_eq!(state.entries()[0].text, "next");
}
