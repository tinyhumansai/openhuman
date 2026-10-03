use super::*;

#[test]
fn prose_is_not_a_tool_call() {
    assert!(!looks_like_unexecuted_tool_call(
        "All done, the file is saved."
    ));
    assert!(!contains_tool_call_payload("All done."));
    assert_eq!(strip_tool_calls_from_response("  plain  "), "  plain  ");
}

#[test]
fn xml_and_json_payloads_are_recognised() {
    assert!(contains_tool_call_payload("<tool_call>{}</tool_call>"));
    assert!(contains_tool_call_payload("{\n  \"tool_calls\": []\n}"));
    assert!(looks_like_unexecuted_tool_call("{\"tool_use\": 1}"));
    assert!(!contains_tool_call_payload("{\"tool_use\": 1}"));
}

#[test]
fn stripping_keeps_only_the_prose() {
    let text = "Before\n<tool_call>{\"name\":\"x\"}</tool_call>\n\n\n\nAfter\n{\"tool_calls\": []}";
    assert_eq!(strip_tool_calls_from_response(text), "Before\n\n\nAfter");
}

#[test]
fn an_unclosed_tag_drops_the_tail() {
    assert_eq!(
        strip_tool_calls_from_response("Keep <tool_call>{\"a\":1"),
        "Keep"
    );
}

#[test]
fn a_pure_payload_strips_to_empty() {
    assert_eq!(
        strip_tool_calls_from_response("<tool_call>{}</tool_call>"),
        ""
    );
}
