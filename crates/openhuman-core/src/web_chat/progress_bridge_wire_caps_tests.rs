use super::*;

#[test]
fn cap_wire_output_passes_through_small_payloads() {
    let s = "small tool result".to_string();
    assert_eq!(cap_wire_output(s.clone()), s);
}

#[test]
fn cap_wire_output_truncates_large_payloads_on_char_boundary() {
    // A multibyte payload past the cap: result stays valid UTF-8, is shorter
    // than the input, and carries the truncation marker.
    let big = "é".repeat(MAX_WIRE_SUBAGENT_OUTPUT); // 2 bytes each → well over cap
    let capped = cap_wire_output(big.clone());
    assert!(capped.len() < big.len());
    assert!(capped.contains("[truncated"));
    // Truncation landed on a char boundary (no replacement char / panic).
    assert!(capped.starts_with('é'));
    // The final payload (content + marker) must honor the wire cap.
    assert!(capped.len() <= MAX_WIRE_SUBAGENT_OUTPUT);
}

#[test]
fn cap_wire_args_passes_through_small_payloads() {
    let args = serde_json::json!({"query": "hello"});
    assert_eq!(cap_wire_args(Some(args.clone())), Some(args));
}

#[test]
fn cap_wire_args_drops_null() {
    assert_eq!(cap_wire_args(Some(serde_json::Value::Null)), None);
    assert_eq!(cap_wire_args(None), None);
}

#[test]
fn cap_wire_args_truncates_oversized_payload_to_a_marker_string() {
    let big = serde_json::json!({ "body": "x".repeat(MAX_WIRE_SUBAGENT_OUTPUT) });
    let capped = cap_wire_args(Some(big)).expect("oversized args still forwarded");
    let rendered = capped.as_str().expect("degrades to a string, not JSON");
    assert!(rendered.len() <= MAX_WIRE_SUBAGENT_OUTPUT);
    assert!(rendered.contains("truncated"));
}
