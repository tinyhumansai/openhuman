use super::*;

// ── extract / cleanup helpers ───────────────────────────────────────

#[test]
fn fenced_block_is_preferred_over_raw_braces() {
    let text = "preamble { \"other\": 1 } middle\n```json\n{\n  \"action\": \"drop\",\n  \"reason\": \"test\"\n}\n```\ntrailing notes";
    let slice = extract_json_slice(text).unwrap();
    assert!(slice.contains("\"action\""));
    assert!(slice.contains("\"reason\": \"test\""));
    assert!(!slice.contains("middle"));
}

#[test]
fn bare_brace_object_is_extracted_when_no_fence() {
    let text = "Here is my verdict: { \"action\": \"drop\", \"reason\": \"test\" } — thanks!";
    let slice = extract_json_slice(text).unwrap();
    assert!(slice.contains("\"action\""));
}

#[test]
fn last_of_multiple_braces_wins() {
    let text = "{ \"action\": \"escalate\", \"reason\": \"first\" } and then { \"action\": \"drop\", \"reason\": \"second\" }";
    let slice = extract_json_slice(text).unwrap();
    assert!(slice.contains("\"second\""));
    assert!(!slice.contains("\"first\""));
}

#[test]
fn brace_inside_string_does_not_break_matching() {
    let text = "{ \"action\": \"drop\", \"reason\": \"has } and { chars\" }";
    let slice = extract_json_slice(text).unwrap();
    assert!(slice.contains("has } and { chars"));
}

#[test]
fn trailing_commas_are_stripped() {
    let src = "{ \"a\": 1, \"b\": [1, 2,], }";
    assert_eq!(strip_trailing_commas(src), "{ \"a\": 1, \"b\": [1, 2] }");
}

#[test]
fn trailing_comma_inside_string_is_left_alone() {
    let src = "{ \"reason\": \"a, b, c,\" }";
    assert_eq!(strip_trailing_commas(src), src);
}

#[test]
fn action_value_is_lowercased() {
    let src = "{\"action\": \"Drop\", \"reason\": \"x\"}";
    let out = lowercase_action_value(src);
    assert!(out.contains("\"action\": \"drop\""));
}

#[test]
fn other_string_values_are_not_lowercased() {
    let src = "{\"action\": \"DROP\", \"reason\": \"X Y Z\"}";
    let out = lowercase_action_value(src);
    assert!(out.contains("\"action\": \"drop\""));
    assert!(out.contains("\"reason\": \"X Y Z\""));
}

// ── full parse_triage_decision ──────────────────────────────────────

#[test]
fn parses_clean_fenced_drop() {
    let reply =
        "Here's my verdict:\n```json\n{\"action\":\"drop\",\"reason\":\"duplicate event\"}\n```\n";
    let d = parse_triage_decision(reply).unwrap();
    assert_eq!(d.action, TriageAction::Drop);
    assert_eq!(d.reason, "duplicate event");
    assert!(d.target_agent.is_none());
    assert!(d.prompt.is_none());
}

#[test]
fn parses_unfenced_json_with_prose_before() {
    let reply = "I think this one needs human attention.\n\n{\"action\":\"escalate\",\"target_agent\":\"orchestrator\",\"prompt\":\"read the email and draft a reply\",\"reason\":\"complex request\"}";
    let d = parse_triage_decision(reply).unwrap();
    assert_eq!(d.action, TriageAction::Escalate);
    assert_eq!(d.target_agent.as_deref(), Some("orchestrator"));
    assert_eq!(
        d.prompt.as_deref(),
        Some("read the email and draft a reply")
    );
}

#[test]
fn parses_react_with_trailing_comma() {
    let reply = "```\n{\n  \"action\": \"react\",\n  \"target_agent\": \"trigger_reactor\",\n  \"prompt\": \"send ack\",\n  \"reason\": \"one-step ack needed\",\n}\n```";
    let d = parse_triage_decision(reply).unwrap();
    assert_eq!(d.action, TriageAction::React);
    assert_eq!(d.target_agent.as_deref(), Some("trigger_reactor"));
}

#[test]
fn parses_uppercase_action_field() {
    let reply = "{\"action\":\"DROP\",\"reason\":\"noise\"}";
    let d = parse_triage_decision(reply).unwrap();
    assert_eq!(d.action, TriageAction::Drop);
}

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
fn rejects_reply_with_no_json_at_all() {
    let reply = "I don't feel like answering today";
    let err = parse_triage_decision(reply).unwrap_err();
    assert!(matches!(err, ParseError::NoJsonObject));
}

#[test]
fn rejects_non_parseable_json() {
    let reply = "{\"action\": not_a_string}";
    let err = parse_triage_decision(reply).unwrap_err();
    assert!(matches!(err, ParseError::InvalidJson(_)));
}

#[test]
fn prefers_last_fenced_block() {
    let reply = "```json\n{\"action\":\"escalate\",\"target_agent\":\"orchestrator\",\"prompt\":\"first\",\"reason\":\"a\"}\n```\nactually scratch that:\n```json\n{\"action\":\"drop\",\"reason\":\"never mind\"}\n```";
    let d = parse_triage_decision(reply).unwrap();
    assert_eq!(d.action, TriageAction::Drop);
    assert_eq!(d.reason, "never mind");
}
