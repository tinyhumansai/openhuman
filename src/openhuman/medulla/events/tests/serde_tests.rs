//! Tests for [`SessionEvent`] JSON serialization: full round-trips across every
//! kind, and the deserialize tolerance rules (unknown kinds, missing fields,
//! empty-string normalization, and error cases).

use serde_json::json;

use crate::openhuman::medulla::events::*;

#[test]
fn unknown_kind_round_trips() {
    let json = r#"{"kind":"weird_kind","payload":42}"#;
    let ev: SessionEvent = serde_json::from_str(json).unwrap();
    match &ev {
        SessionEvent::Unknown { kind, data } => {
            assert_eq!(kind, "weird_kind");
            assert_eq!(data.get("payload").unwrap(), &json!(42));
        }
        _ => panic!("expected unknown"),
    }
    let back = serde_json::to_value(&ev).unwrap();
    assert_eq!(back.get("kind").unwrap(), &json!("weird_kind"));
    assert_eq!(back.get("payload").unwrap(), &json!(42));
}

#[test]
fn known_event_round_trips() {
    let ev = SessionEvent::InferenceEnd {
        tier: "reasoning".into(),
        op: "execute_step".into(),
        model: Some("gpt".into()),
        duration_ms: 120,
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        }),
        content: None,
        reasoning: None,
        tool_calls: None,
    };
    let s = serde_json::to_string(&ev).unwrap();
    let back: SessionEvent = serde_json::from_str(&s).unwrap();
    assert_eq!(ev, back);
}

/// One representative JSON per kind, exercising every deserialize arm.
fn one_of_each() -> Vec<(&'static str, SessionEvent)> {
    vec![
        (
            "inference_start",
            SessionEvent::InferenceStart {
                tier: "orchestrator".into(),
                op: "orchestrate".into(),
                model: Some("m".into()),
            },
        ),
        (
            "inference_end",
            SessionEvent::InferenceEnd {
                tier: "reasoning".into(),
                op: "step".into(),
                model: None,
                duration_ms: 5,
                usage: None,
                content: Some("c".into()),
                reasoning: Some("r".into()),
                tool_calls: Some(vec![ToolCall {
                    name: "grep".into(),
                    args: json!({"q": 1}),
                }]),
            },
        ),
        (
            "tool_call_start",
            SessionEvent::ToolCallStart {
                index: 2,
                name: "read".into(),
            },
        ),
        (
            "tool_call_delta",
            SessionEvent::ToolCallDelta {
                index: 2,
                args_delta: "{\"a\":".into(),
            },
        ),
        (
            "assistant_delta",
            SessionEvent::AssistantDelta { delta: "x".into() },
        ),
        (
            "reasoning_delta",
            SessionEvent::ReasoningDelta { delta: "y".into() },
        ),
        (
            "task_start",
            SessionEvent::TaskStart {
                task_id: "t1".into(),
                instruction: "do".into(),
                depth: 2,
                agent_id: Some("dev".into()),
                contract: None,
            },
        ),
        (
            "task_event",
            SessionEvent::TaskEvent {
                task_id: "t1".into(),
                event_kind: "text".into(),
                content: "hi".into(),
                harness: Some("codex".into()),
            },
        ),
        (
            "task_attention",
            SessionEvent::TaskAttention {
                task_id: "t1".into(),
                reason: "confirm".into(),
                content: "proceed?".into(),
                question_id: Some("q1".into()),
            },
        ),
        (
            "task_complete",
            SessionEvent::TaskComplete {
                digest: TaskDigest {
                    task_id: "t1".into(),
                    status: "done".into(),
                    digest: "d".into(),
                    result_ref: Some(json!({"ref": 1})),
                    usage: Some(Usage {
                        input_tokens: 1,
                        output_tokens: 2,
                        ..Default::default()
                    }),
                    depth: 2,
                    contract: None,
                    evidence: None,
                },
            },
        ),
        (
            "trace",
            SessionEvent::Trace {
                entry: NodeTrace {
                    node: "orchestrate".into(),
                    ms: 12,
                    tool: Some("grep".into()),
                    op: None,
                },
            },
        ),
        (
            "error",
            SessionEvent::Error {
                source: "cycle".into(),
                message: "boom".into(),
            },
        ),
        (
            "cycle_start",
            SessionEvent::CycleStart {
                cycle_id: "c1".into(),
            },
        ),
        (
            "cycle_end",
            SessionEvent::CycleEnd {
                cycle_id: "c1".into(),
                pass_count: 3,
                duration_ms: 99,
            },
        ),
        (
            "agent_status",
            SessionEvent::AgentStatus {
                agent_id: "dev".into(),
                availability: "online".into(),
                detail: Some("idle".into()),
            },
        ),
        (
            "session_event",
            SessionEvent::SessionEvent {
                agent_id: "m1".into(),
                session_id: "s1".into(),
                event_kind: "stdout".into(),
                content: "log".into(),
            },
        ),
        (
            "peer_session",
            SessionEvent::PeerSession {
                agent_id: "m1".into(),
                session_id: "s1".into(),
                state: "working".into(),
                harness: Some("codex".into()),
            },
        ),
        ("user", SessionEvent::User { body: "hey".into() }),
        ("assistant", SessionEvent::Assistant { body: "yo".into() }),
        (
            "effect",
            SessionEvent::Effect {
                effect: json!({"kind": "send"}),
            },
        ),
    ]
}

#[test]
fn every_kind_round_trips_and_reports_kind() {
    for (kind, ev) in one_of_each() {
        assert_eq!(ev.kind(), kind, "kind() mismatch for {kind}");
        let s = serde_json::to_string(&ev).unwrap();
        let back: SessionEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(ev, back, "round-trip mismatch for {kind}");
    }
}

#[test]
fn empty_object_is_unknown_with_empty_kind() {
    let ev: SessionEvent = serde_json::from_str("{}").unwrap();
    assert!(matches!(&ev, SessionEvent::Unknown { kind, .. } if kind.is_empty()));
    assert_eq!(ev.kind(), "");
}

#[test]
fn non_object_json_is_a_deserialize_error() {
    assert!(serde_json::from_str::<SessionEvent>("[1,2,3]").is_err());
    assert!(serde_json::from_str::<SessionEvent>("42").is_err());
}

#[test]
fn task_complete_without_digest_errors() {
    assert!(serde_json::from_str::<SessionEvent>(r#"{"kind":"task_complete"}"#).is_err());
}

#[test]
fn trace_without_entry_errors() {
    assert!(serde_json::from_str::<SessionEvent>(r#"{"kind":"trace"}"#).is_err());
}

#[test]
fn opt_str_filters_empty_to_none() {
    // An empty `model` string decodes to `None`, not `Some("")`.
    let ev: SessionEvent =
        serde_json::from_str(r#"{"kind":"inference_start","tier":"r","op":"o","model":""}"#)
            .unwrap();
    assert!(matches!(
        ev,
        SessionEvent::InferenceStart { model: None, .. }
    ));
}

#[test]
fn serialize_drops_null_fields() {
    // A model-less inference_start must not carry a `"model":null` key.
    let ev = SessionEvent::InferenceStart {
        tier: "r".into(),
        op: "o".into(),
        model: None,
    };
    let v = serde_json::to_value(&ev).unwrap();
    assert!(v.get("model").is_none(), "null model should be dropped");
    assert_eq!(v.get("kind").unwrap(), &json!("inference_start"));
}

#[test]
fn effect_decode_defaults_to_null_when_missing() {
    let ev: SessionEvent = serde_json::from_str(r#"{"kind":"effect"}"#).unwrap();
    assert!(matches!(ev, SessionEvent::Effect { effect } if effect.is_null()));
}

#[test]
fn envelope_round_trips() {
    let e = EventEnvelope {
        seq: 7,
        at: 123,
        event: SessionEvent::User { body: "hi".into() },
    };
    let s = serde_json::to_string(&e).unwrap();
    let back: EventEnvelope = serde_json::from_str(&s).unwrap();
    assert_eq!(e, back);
}

#[test]
fn tool_call_defaults_args_to_null() {
    let tc: ToolCall = serde_json::from_str(r#"{"name":"grep"}"#).unwrap();
    assert_eq!(tc.name, "grep");
    assert!(tc.args.is_null());
}

#[test]
fn worker_contract_and_evidence_round_trip_on_task_events() {
    let start = json!({
        "kind": "task_start",
        "taskId": "lane-1",
        "instruction": "Implement.",
        "depth": 2,
        "contract": {
            "outcome": "Ship the parser",
            "permittedPaths": ["src/parser/**"],
            "nonGoals": ["No UI"],
            "verifyCommands": ["cargo test parser"],
            "terminalCondition": "tests green"
        }
    });
    let event: SessionEvent = serde_json::from_value(start.clone()).unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), start);

    let complete = json!({
        "kind": "task_complete",
        "digest": {
            "taskId": "lane-1",
            "status": "done",
            "digest": "implemented",
            "depth": 2,
            "contract": {
                "outcome": "Ship the parser",
                "permittedPaths": ["src/parser/**"]
            },
            "evidence": [
                {"gate": "verify_examples", "ok": true, "summary": "VERIFIED"},
                {"command": "cargo test parser", "ok": true, "summary": "12 passed"}
            ]
        }
    });
    let event: SessionEvent = serde_json::from_value(complete.clone()).unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), complete);
}
