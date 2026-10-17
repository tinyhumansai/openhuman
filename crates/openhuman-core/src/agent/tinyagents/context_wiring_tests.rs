use super::*;
use tinyagents_harness::turn_outcome::ToolCallOutcome;

#[test]
fn outcome_source_returns_captured_content_by_call_id() {
    let sink: ToolOutcomeSink = Default::default();
    let source = OutcomeSinkSource(sink.clone());
    assert!(source.content_for("missing").unwrap().is_none());
    sink.lock().unwrap().push(ToolCallOutcome {
        call_id: "c1".into(),
        name: "t".into(),
        arguments: serde_json::Value::Null,
        success: true,
        content: "body".into(),
        duration_ms: 0,
    });
    assert_eq!(source.content_for("c1").unwrap().as_deref(), Some("body"));
}

#[test]
fn budget_notice_thresholds_stay_at_half_and_eighty_percent() {
    assert_eq!(BUDGET_NOTICE_THRESHOLDS, [0.5, 0.8]);
}
