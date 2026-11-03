use super::*;
use serde_json::json;

fn view(mut event: Value) -> WebEventView {
    let obj = event.as_object_mut().unwrap();
    obj.entry("client_id").or_insert(json!("c1"));
    obj.entry("thread_id").or_insert(json!("t1"));
    obj.entry("request_id").or_insert(json!("r1"));
    obj.entry("seq").or_insert(json!(7));
    let parsed: WebChannelEvent = serde_json::from_value(event).expect("valid event");
    WebEventView::from_event(&parsed)
}

fn sub() -> Value {
    json!({"task_id": "k1", "display_name": "Explore", "elapsed_ms": 40, "output": "sub-out"})
}

fn assert_sub(v: &WebEventView) {
    let s = v.subagent.as_ref().expect("subagent projected");
    assert_eq!(s.task_id.as_deref(), Some("k1"));
    assert_eq!(s.display_name.as_deref(), Some("Explore"));
    assert_eq!(s.elapsed_ms, Some(40));
    assert_eq!(s.output.as_deref(), Some("sub-out"));
}

#[test]
fn context_fields_and_event_name_are_projected() {
    let v = view(json!({"event": "text_delta", "delta": "hi"}));
    assert_eq!(v.event, "text_delta");
    assert_eq!(
        (
            v.client_id.as_str(),
            v.thread_id.as_str(),
            v.request_id.as_str()
        ),
        ("c1", "t1", "r1")
    );
    assert_eq!(v.seq, Some(7));
    assert_eq!(v.delta.as_deref(), Some("hi"));
}

#[test]
fn final_response_variants_expose_full_response() {
    for name in [
        "chat_done",
        "chat_interim",
        "chat_message",
        "chat_segment",
        "proactive_message",
        "voice_speak",
        "channel_outbound",
    ] {
        let parsed = serde_json::from_value::<WebChannelEvent>(json!({
            "event": name, "client_id": "c", "thread_id": "t", "request_id": "r",
            "full_response": "answer"
        }));
        let event = parsed.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            WebEventView::from_event(&event).full_response.as_deref(),
            Some("answer"),
            "{name}"
        );
    }
    let v = view(json!({"event": "chat_done", "full_response": "answer"}));
    assert_eq!(v.full_response.as_deref(), Some("answer"));
}

#[test]
fn chat_error_and_cancel_expose_retry_metadata() {
    let v = view(
        json!({"event": "chat_error", "message": "boom", "error_type": "network",
        "error_retryable": true, "error_retry_after_ms": 1500}),
    );
    assert_eq!(v.message.as_deref(), Some("boom"));
    assert_eq!(v.error_type.as_deref(), Some("network"));
    assert_eq!(v.error_retryable, Some(true));
    assert_eq!(v.error_retry_after_ms, Some(1500));

    let v = view(json!({"event": "chat_cancelled", "cancel_reason": "user"}));
    assert_eq!(v.message.as_deref(), Some("user"));

    let v = view(json!({"event": "user_error", "error_type": "bad_input"}));
    assert_eq!(v.error_type.as_deref(), Some("bad_input"));
}

#[test]
fn deltas_and_iteration_messages_are_projected() {
    assert_eq!(
        view(json!({"event": "thinking_delta", "delta": "hm"}))
            .delta
            .as_deref(),
        Some("hm")
    );
    assert_eq!(
        view(json!({"event": "iteration_start", "message": "round 2"}))
            .message
            .as_deref(),
        Some("round 2")
    );
    let v = view(
        json!({"event": "tool_args_delta", "delta": "{\"a\"", "tool_name": "t",
        "skill_id": "s", "tool_call_id": "tc"}),
    );
    assert_eq!(v.delta.as_deref(), Some("{\"a\""));
    assert_eq!(v.tool_name.as_deref(), Some("t"));
    assert_eq!(v.skill_id.as_deref(), Some("s"));
    assert_eq!(v.tool_call_id.as_deref(), Some("tc"));
    let v = view(json!({"event": "tool_progress", "tool_call_id": "tc", "message": "50%"}));
    assert_eq!(
        (v.tool_call_id.as_deref(), v.message.as_deref()),
        (Some("tc"), Some("50%"))
    );
}

#[test]
fn subagent_deltas_carry_subagent_detail() {
    for name in ["subagent_text_delta", "subagent_thinking_delta"] {
        let v = view(json!({"event": name, "delta": "d", "skill_id": "sk", "subagent": sub()}));
        assert_eq!(v.delta.as_deref(), Some("d"));
        assert_eq!(v.skill_id.as_deref(), Some("sk"));
        assert_sub(&v);
    }
}

#[test]
fn tool_call_and_result_variants_project_all_fields() {
    let v = view(
        json!({"event": "tool_call", "tool_name": "read", "skill_id": "s",
        "args": {"p": 1}, "tool_call_id": "tc", "tool_display_label": "Reading"}),
    );
    assert_eq!(v.tool_name.as_deref(), Some("read"));
    assert_eq!(v.args, Some(json!({"p": 1})));
    assert_eq!(v.tool_display_label.as_deref(), Some("Reading"));

    let v = view(
        json!({"event": "subagent_tool_call", "tool_name": "read", "skill_id": "s",
        "args": {"p": 1}, "tool_call_id": "tc", "tool_display_label": "Reading", "subagent": sub()}),
    );
    assert_eq!(v.tool_call_id.as_deref(), Some("tc"));
    assert_sub(&v);

    let result = json!({"tool_name": "read", "skill_id": "s", "output": "out", "args": {"p": 2},
        "success": false, "tool_call_id": "tc", "failure": {"code": "x"}, "elapsed_ms": 9,
        "tool_display_label": "Read"});
    for (name, with_sub) in [("tool_result", false), ("subagent_tool_result", true)] {
        let mut e = result.clone();
        e["event"] = json!(name);
        if with_sub {
            e["subagent"] = sub();
        }
        let v = view(e);
        assert_eq!(v.output.as_deref(), Some("out"));
        assert_eq!(v.success, Some(false));
        assert_eq!(v.failure, Some(json!({"code": "x"})));
        assert_eq!(v.elapsed_ms, Some(9));
        assert_eq!(v.subagent.is_some(), with_sub);
        if with_sub {
            assert_sub(&v);
        }
    }
}

#[test]
fn approval_and_plan_review_variants_project_prompt_fields() {
    for name in ["approval_request", "plan_review_request"] {
        let v = view(
            json!({"event": name, "tool_name": "shell", "args": {"cmd": "ls"},
            "tool_call_id": "tc", "message": "allow?"}),
        );
        assert_eq!(v.tool_name.as_deref(), Some("shell"), "{name}");
        assert_eq!(v.message.as_deref(), Some("allow?"), "{name}");
        assert_eq!(v.args, Some(json!({"cmd": "ls"})), "{name}");
    }
    for name in ["approval_decided", "plan_review_decided"] {
        let v = view(
            json!({"event": name, "tool_name": "shell", "message": "approved", "tool_call_id": "tc"}),
        );
        assert_eq!(v.message.as_deref(), Some("approved"), "{name}");
        assert_eq!(v.tool_call_id.as_deref(), Some("tc"), "{name}");
    }
}

#[test]
fn subagent_lifecycle_events_project_status() {
    for name in ["subagent_spawned", "subagent_iteration_start"] {
        let v = view(
            json!({"event": name, "tool_name": "Explore", "skill_id": "s",
            "message": "go", "subagent": sub()}),
        );
        assert_eq!(v.message.as_deref(), Some("go"), "{name}");
        assert_sub(&v);
    }
    for name in [
        "subagent_completed",
        "subagent_failed",
        "subagent_awaiting_user",
    ] {
        let v = view(
            json!({"event": name, "tool_name": "Explore", "skill_id": "s",
            "message": "m", "success": true, "subagent": sub()}),
        );
        assert_eq!(v.success, Some(true), "{name}");
        assert_eq!(v.tool_name.as_deref(), Some("Explore"), "{name}");
        assert_sub(&v);
    }
}

#[test]
fn artifact_events_expose_call_id_and_args() {
    for name in ["artifact_ready", "artifact_failed", "artifact_pending"] {
        let parsed = serde_json::from_value::<WebChannelEvent>(json!({
            "event": name, "client_id": "c", "thread_id": "t", "request_id": "r",
            "tool_call_id": "tc", "args": {"a": 1}
        }));
        let event = parsed.unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = WebEventView::from_event(&event);
        assert_eq!(v.tool_call_id.as_deref(), Some("tc"), "{name}");
        assert_eq!(v.args, Some(json!({"a": 1})), "{name}");
    }
}

#[test]
fn unconsumed_events_leave_payload_fields_empty() {
    let v = view(json!({"event": "inference_start"}));
    assert_eq!(v.event, "inference_start");
    assert!(v.delta.is_none() && v.message.is_none() && v.subagent.is_none());
}
