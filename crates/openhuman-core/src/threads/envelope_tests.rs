use super::*;

#[test]
fn counts_collects_named_entries() {
    let counts = counts([("documents", 2), ("entities", 5)]);
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
fn envelope_propagates_counts_and_pagination() {
    let pagination = PaginationMeta {
        limit: 10,
        offset: 0,
        count: 7,
    };
    let value = envelope(
        serde_json::json!({"v": 42}),
        Some(counts([("num_messages", 7)])),
        Some(pagination),
    )
    .into_cli_compatible_json()
    .unwrap();
    assert_eq!(value["meta"]["counts"]["num_messages"], 7);
    assert_eq!(value["meta"]["pagination"]["count"], 7);
}
