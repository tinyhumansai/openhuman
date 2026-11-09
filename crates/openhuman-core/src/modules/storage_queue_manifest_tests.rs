use super::*;

#[test]
fn parses_supported_manifest_and_preserves_queue_policy() {
    let manifest = QueueManifest::from_json(
        r#"{"$schema":"https://example.test/queues.schema.json","version":1,"queues":[{"name":"inbox","description":"inbound work","owner":"worker","max_attempts":3,"dedupe_window_ms":60000,"visibility_ms":30000}]}"#,
    )
    .unwrap();
    let declaration = manifest.get("inbox").unwrap();
    assert_eq!(declaration.spec().max_attempts, 3);
    assert_eq!(declaration.spec().dedupe_window_ms, 60_000);
    assert_eq!(declaration.visibility_ms, Some(30_000));
}

#[test]
fn rejects_unknown_fields_duplicate_names_and_invalid_future_versions() {
    for text in [
        r#"{"version":1,"queues":[],"fallback":"memory"}"#,
        r#"{"version":2,"queues":[]}"#,
        r#"{"version":1,"queues":[{"name":"q","max_attempts":2},{"name":"q","max_attempts":3}]}"#,
        r#"{"version":1,"queues":[{"name":"Bad Name"}]}"#,
    ] {
        assert!(QueueManifest::from_json(text).is_err(), "{text}");
    }
}

#[test]
fn allows_old_manifest_defaults_but_rejects_zero_attempts_or_visibility() {
    let manifest =
        QueueManifest::from_json(r#"{"version":1,"queues":[{"name":"legacy"}]}"#).unwrap();
    assert_eq!(manifest.get("legacy").unwrap().spec().max_attempts, 8);
    for text in [
        r#"{"version":1,"queues":[{"name":"q","max_attempts":0}]}"#,
        r#"{"version":1,"queues":[{"name":"q","visibility_ms":0}]}"#,
    ] {
        assert!(QueueManifest::from_json(text).is_err(), "{text}");
    }
}
