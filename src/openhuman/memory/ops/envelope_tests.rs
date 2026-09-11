use super::*;

#[test]
fn memory_counts_collects_named_entries() {
    let counts = memory_counts([("documents", 2), ("entities", 5)]);
    assert_eq!(counts.get("documents"), Some(&2));
    assert_eq!(counts.get("entities"), Some(&5));
}

#[test]
fn envelope_wraps_data_with_meta() {
    let outcome = envelope(serde_json::json!({"ok": true}), None, None);
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["data"]["ok"], true);
    assert!(value["meta"]["request_id"].as_str().is_some());
    assert!(value["error"].is_null());
}

#[test]
fn error_envelope_wraps_code_and_message() {
    let outcome: RpcOutcome<ApiEnvelope<serde_json::Value>> =
        error_envelope("bad_request", "boom".to_string());
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["error"]["code"], "bad_request");
    assert_eq!(value["error"]["message"], "boom");
    assert!(value["data"].is_null());
}
