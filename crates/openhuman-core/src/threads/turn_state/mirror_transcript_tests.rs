use super::*;

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
