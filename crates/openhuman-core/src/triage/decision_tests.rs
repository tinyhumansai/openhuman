use super::*;

#[test]
fn rejects_escalate_without_target_agent() {
    let reply = "{\"action\":\"escalate\",\"reason\":\"complex\"}";
    let err = parse_triage_decision(reply).unwrap_err();
    assert!(matches!(
        err,
        ParseError::MissingTarget { action: "escalate" }
    ));
}

#[test]
fn rejects_react_without_prompt() {
    let reply = "{\"action\":\"react\",\"target_agent\":\"trigger_reactor\",\"reason\":\"x\"}";
    let err = parse_triage_decision(reply).unwrap_err();
    assert!(matches!(err, ParseError::MissingTarget { action: "react" }));
}

#[test]
fn parses_valid_react_and_escalate_decisions_with_host_fields() {
    let cases = [
        (
            r#"{"action":"react","target_agent":"trigger_reactor","prompt":"send acknowledgement","reason":"one-step response"}"#,
            TriageAction::React,
            "trigger_reactor",
            "send acknowledgement",
        ),
        (
            r#"{"action":"escalate","target_agent":"orchestrator","prompt":"draft a reply","reason":"complex request"}"#,
            TriageAction::Escalate,
            "orchestrator",
            "draft a reply",
        ),
    ];

    for (reply, expected_action, expected_target, expected_prompt) in cases {
        let decision = parse_triage_decision(reply).expect("valid host decision");
        assert_eq!(decision.action, expected_action);
        assert_eq!(decision.target_agent.as_deref(), Some(expected_target));
        assert_eq!(decision.prompt.as_deref(), Some(expected_prompt));
    }
}

#[test]
fn rejects_non_parseable_json() {
    let reply = "{\"action\": not_a_string}";
    let err = parse_triage_decision(reply).unwrap_err();
    assert!(matches!(err, ParseError::InvalidJson(_)));
}

#[test]
fn owner_relaxed_json_recovery_preserves_action_normalization() {
    let reply = "{action: \"DROP\", reason: \"recovered\",}";
    let decision = parse_triage_decision(reply).unwrap();
    assert_eq!(decision.action, TriageAction::Drop);
    assert_eq!(decision.reason, "recovered");
}
