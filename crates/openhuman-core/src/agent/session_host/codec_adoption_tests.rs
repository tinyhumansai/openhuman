use super::*;
use tinyagents_runtime::ResumeMode;

fn meta() -> tinyagents_session::transcript::TranscriptMeta {
    tinyagents_session::transcript::TranscriptMeta {
        session_id: None,
        parent_session_id: None,
        agent_name: "adapter-test".into(),
        agent_id: Some("adapter-test".into()),
        agent_type: Some("root".into()),
        dispatcher: "test".into(),
        provider: None,
        model: None,
        created: chrono::Utc::now().to_rfc3339(),
        updated: chrono::Utc::now().to_rfc3339(),
        turn_count: 0,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: None,
        task_id: None,
    }
}

#[test]
fn codec_still_rejects_an_orphan_result_after_a_user_message() {
    let transcript = tinyagents_session::transcript::SessionTranscript {
        meta: meta(),
        messages: [
            serde_json::json!({"role":"system","content":"frozen"}),
            serde_json::json!({"role":"tool","id":"legacy-head","content":"trimmed"}),
            serde_json::json!({"role":"user","content":"continue"}),
            serde_json::json!({"role":"tool","id":"later-orphan","content":"invalid"}),
        ]
        .into_iter()
        .map(|row| serde_json::from_value(row).unwrap())
        .collect(),
        tools: None,
    };
    let error = OpenHumanTranscriptCodec
        .decode_history(&transcript)
        .unwrap_err();
    assert!(error.to_string().contains("later-orphan"), "{error}");
    assert_eq!(transcript.messages[1].id.as_deref(), Some("legacy-head"));
}

#[test]
fn codec_retains_authoritative_row_metadata_after_legacy_head_adoption() {
    let transcript = tinyagents_session::transcript::SessionTranscript {
        meta: meta(),
        messages: [
            serde_json::json!({"role":"system","content":"frozen"}),
            serde_json::json!({"role":"tool","id":"legacy-head","content":"trimmed"}),
            serde_json::json!({"role":"user","content":"continue","request_id":"original",
                "extra_metadata":{"retained":"product-value"}}),
        ]
        .into_iter()
        .map(|row| serde_json::from_value(row).unwrap())
        .collect(),
        tools: None,
    };
    let previous = OpenHumanTranscriptCodec
        .decode_history(&transcript)
        .unwrap();
    let mut next = previous.clone();
    next.push(Message::assistant("done"));
    let rows = OpenHumanTranscriptCodec
        .reconcile(
            &transcript.messages,
            &previous,
            &next,
            &TranscriptTurnOptions {
                request_id: Some("new-turn".into()),
                thread_id: None,
                stream: false,
                resume: ResumeMode::Never,
                context: OpenHumanRunContext::new(),
            },
        )
        .unwrap();
    assert_eq!(rows[1].role, "user");
    assert_eq!(rows[1].request_id.as_deref(), Some("original"));
    assert_eq!(
        rows[1].extra_metadata,
        Some(serde_json::json!({"retained":"product-value"}))
    );
    assert_eq!(rows[2].request_id.as_deref(), Some("new-turn"));
    assert_eq!(transcript.messages[1].id.as_deref(), Some("legacy-head"));
}

/// Providers reuse tool-call ids across turns (`call_0`, `call_1`, ...). A
/// failure in this turn must not retroactively mark an earlier turn's
/// successful result with the same id as failed: that rewrites a persisted row,
/// so the commit no longer extends the transcript and opens a new generation.
#[test]
fn failed_tool_outcome_does_not_mark_an_earlier_turns_result_with_the_same_call_id() {
    let options = |context| TranscriptTurnOptions {
        request_id: Some("turn".into()),
        thread_id: None,
        stream: false,
        resume: ResumeMode::Never,
        context,
    };
    let first = vec![Message::user("one"), Message::tool("call_1", "scheduled")];
    let prior = OpenHumanTranscriptCodec
        .reconcile(&[], &[], &first, &options(OpenHumanRunContext::new()))
        .unwrap();
    assert!(prior[1].tool_failure.is_none());

    let context = OpenHumanRunContext::new();
    context.session_sidecar.lock().unwrap().tool_outcomes.push(
        tinyagents_harness::turn_outcome::ToolCallOutcome {
            call_id: "call_1".into(),
            name: "cron".into(),
            arguments: serde_json::json!({}),
            success: false,
            content: "refused".into(),
            duration_ms: 0,
        },
    );
    let mut next = first.clone();
    next.push(Message::user("two"));
    next.push(Message::tool("call_1", "refused"));
    let rows = OpenHumanTranscriptCodec
        .reconcile(&prior, &first, &next, &options(context))
        .unwrap();
    assert!(
        rows[1].tool_failure.is_none(),
        "an earlier turn's result keeps its recorded status"
    );
    assert!(rows[3].tool_failure.as_ref().is_some_and(|f| f.failed));
}
