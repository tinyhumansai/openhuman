//! Maps the agent's per-turn [`AgentProgress`] stream onto the
//! `tinyplace.harness.session.v2` envelope kinds openhuman already emits
//! through its tinyplace wrapper.
//!
//! We deliberately reuse the tinyplace v2 types (`SessionEnvelopeV2`,
//! `HarnessEvent`, `HarnessEventKind`, and the per-kind payload structs) rather
//! than re-deriving the envelope schema — the produced envelopes are wire-
//! identical to the ones a wrapped CLI session forwards, so a medulla operator
//! consumes one stream shape regardless of how the session was driven.

use tinyplace::types::{
    ApprovalRequestPayload, HarnessEvent, HarnessEventKind, HarnessScope, SessionEnvelopeV2,
    StatusPayload, TextPayload, ToolCallPayload, ToolResultPayload, SESSION_ENVELOPE_VERSION_V2,
};

use crate::openhuman::agent::progress::AgentProgress;

/// `role` stamped on every openhuman-produced event: these are agent-side
/// stream frames (`owner` is reserved for `user_prompt`, which the agent never
/// emits about itself).
const AGENT_ROLE: &str = "agent";

/// Translate one [`AgentProgress`] event into a typed v2 event kind.
///
/// Returns `None` for progress variants that carry no user-facing stream frame
/// (cost rollups, per-call token accounting, arg-delta fragments, …) so the
/// forwarded stream stays close to the `agent_message / agent_thinking /
/// tool_call / tool_result / status / approval_request / error` vocabulary the
/// spec enumerates.
pub fn progress_to_event_kind(progress: &AgentProgress) -> Option<HarnessEventKind> {
    let kind = match progress {
        AgentProgress::TurnStarted => HarnessEventKind::Status(StatusPayload {
            state: "running".to_string(),
            detail: "turn started".to_string(),
            active_call_id: None,
        }),
        AgentProgress::IterationStarted {
            iteration,
            max_iterations,
        } => HarnessEventKind::Status(StatusPayload {
            state: "running".to_string(),
            detail: format!("iteration {iteration}/{max_iterations}"),
            active_call_id: None,
        }),
        AgentProgress::TextDelta { delta, .. } => HarnessEventKind::AgentMessage(TextPayload {
            text: delta.clone(),
        }),
        AgentProgress::ThinkingDelta { delta, .. } => {
            HarnessEventKind::AgentThinking(TextPayload {
                text: delta.clone(),
            })
        }
        AgentProgress::ToolCallStarted {
            call_id,
            tool_name,
            arguments,
            display_label,
            ..
        } => HarnessEventKind::ToolCall(ToolCallPayload {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            tool_kind: "other".to_string(),
            display: display_label.clone().unwrap_or_else(|| tool_name.clone()),
            input: arguments.clone(),
        }),
        AgentProgress::ToolCallCompleted {
            call_id,
            success,
            output,
            output_chars,
            ..
        } => HarnessEventKind::ToolResult(ToolResultPayload {
            call_id: call_id.clone(),
            ok: *success,
            exit_code: None,
            is_error: !*success,
            output: output.clone(),
            output_bytes: *output_chars as i64,
        }),
        AgentProgress::SubagentAwaitingUser {
            task_id, question, ..
        } => HarnessEventKind::ApprovalRequest(ApprovalRequestPayload {
            call_id: Some(task_id.clone()),
            tool_name: "subagent".to_string(),
            display: question.clone(),
            reason: None,
        }),
        AgentProgress::TurnCompleted { .. } => HarnessEventKind::Status(StatusPayload {
            state: "idle".to_string(),
            detail: "turn completed".to_string(),
            active_call_id: None,
        }),
        // Everything else (arg deltas, cost/usage rollups, per-call model
        // accounting, subagent-internal frames, task-board writes, raw
        // TurnContent) carries no distinct stream frame in this vocabulary.
        _ => return None,
    };
    Some(kind)
}

/// Wrap a typed [`HarnessEventKind`] in a full [`SessionEnvelopeV2`] anchored to
/// `session_id`, ready to serialize into a `medulla:task_envelope` frame.
///
/// `seq` is the monotonic per-session ordering counter; `ts` is an ISO-8601
/// timestamp.
pub fn envelope_for_kind(session_id: &str, seq: i64, kind: &HarnessEventKind) -> SessionEnvelopeV2 {
    // `HarnessEventKind` is adjacently tagged (`{ "kind": .., "payload": .. }`),
    // so serializing it yields exactly the `kind`/`payload` pair `HarnessEvent`
    // stores — extract them rather than hand-writing the discriminator strings.
    let tagged = serde_json::to_value(kind).unwrap_or_default();
    let kind_str = tagged
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let payload = tagged
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    SessionEnvelopeV2 {
        envelope_version: SESSION_ENVELOPE_VERSION_V2.to_string(),
        version: 2,
        scope: HarnessScope {
            scope_type: "session".to_string(),
            wrapper_session_id: session_id.to_string(),
            harness_session_id: session_id.to_string(),
            ..Default::default()
        },
        event: HarnessEvent {
            id: format!("{session_id}-{seq}"),
            seq,
            ts: chrono::Utc::now().to_rfc3339(),
            role: AGENT_ROLE.to_string(),
            kind: kind_str,
            payload,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Convenience: build a bare `status` envelope (used to bookend a task run).
pub fn status_envelope(session_id: &str, seq: i64, state: &str, detail: &str) -> SessionEnvelopeV2 {
    envelope_for_kind(
        session_id,
        seq,
        &HarnessEventKind::Status(StatusPayload {
            state: state.to_string(),
            detail: detail.to_string(),
            active_call_id: None,
        }),
    )
}

/// Convenience: build an `error` envelope.
pub fn error_envelope(session_id: &str, seq: i64, message: &str, fatal: bool) -> SessionEnvelopeV2 {
    envelope_for_kind(
        session_id,
        seq,
        &HarnessEventKind::Error(tinyplace::types::ErrorPayload {
            message: message.to_string(),
            fatal,
        }),
    )
}

#[cfg(test)]
mod tests {
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
    fn envelope_is_valid_v2_and_round_trips_through_the_wire() {
        let kind = HarnessEventKind::AgentMessage(TextPayload {
            text: "done".to_string(),
        });
        let env = envelope_for_kind("sess-1", 7, &kind);
        assert!(env.is_valid_v2());
        assert_eq!(env.event.kind, "agent_message");
        assert_eq!(env.event.seq, 7);

        // Serialize → parse back through the tinyplace v2 decoder and confirm
        // the typed kind survives the round-trip.
        let wire = serde_json::to_string(&env).unwrap();
        let parsed = SessionEnvelopeV2::parse(&wire).expect("valid v2 wire");
        match parsed.event.decoded() {
            HarnessEventKind::AgentMessage(t) => assert_eq!(t.text, "done"),
            other => panic!("unexpected decoded kind: {other:?}"),
        }
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
}
