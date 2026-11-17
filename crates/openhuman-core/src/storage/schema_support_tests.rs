use super::*;
use serde_json::json;

#[test]
fn rfc3339_becomes_epoch_milliseconds() {
    assert_eq!(rfc3339_ms("1970-01-01T00:00:01.500Z"), Some(1500));
    assert_eq!(rfc3339_ms("not a time"), None);
}

#[test]
fn body_drops_absent_and_null_fields() {
    let from = json!({ "a": 1, "b": null });
    let body = Body::new()
        .copy(&from, "a")
        .copy(&from, "b")
        .copy(&from, "c")
        .set_some("d", None::<i64>)
        .set_some("e", Some(2))
        .copy_as(&from, "a", "z")
        .build();
    assert_eq!(body, json!({ "a": 1, "e": 2, "z": 1 }));
}

#[test]
fn a_natural_key_escapes_separators() {
    assert_eq!(key_of("plain").unwrap(), "plain");
    assert_eq!(key_of("a/b").unwrap(), "a%2Fb");
    assert!(key_of("").is_err());
}

#[test]
fn composite_keys_round_trip_reserved_and_unicode_parts() {
    let encoded = key_of_parts(["a/b", "c%~é"]).unwrap();
    assert_eq!(encoded, "a%2Fb/c%25%7Eé");
    assert_eq!(Key::new(encoded).unwrap().split().unwrap(), ["a/b", "c%~é"]);
    assert!(key_of_parts(["a", ""]).is_err());
}

#[test]
fn fresh_record_key_is_accepted_by_the_storage_key_type() {
    assert!(Key::new(fresh_key()).is_ok());
}

#[test]
fn keys_convert_at_every_depth_and_back() {
    let renames = [("createdAtMillis", "created_ms")];
    let camel = json!({
        "id": "a",
        "createdAtMillis": 5,
        "versions": [{ "authorId": "x", "file": { "fileRoot": "r" } }],
    });
    let snake = snake_keys(&camel, &renames);
    assert_eq!(
        snake,
        json!({
            "id": "a",
            "created_ms": 5,
            "versions": [{ "author_id": "x", "file": { "file_root": "r" } }],
        })
    );
    assert_eq!(camel_keys(&snake, &renames), camel);
}

#[test]
fn owner_conversion_preserves_driver_times_and_absolute_expiry() {
    use crate::storage::import::Record;

    let source = Record {
        collection: "legacy".into(),
        id: "source-id".into(),
        doc: json!({"legacy":true}),
        created_ms: Some(100),
        schema: Some(7),
        expires_ms: Some(900),
        updated_ms: Some(200),
    };
    let converted = map_legacy_collection(vec![source], "legacy", |rows| {
        rows.into_iter()
            .map(|row| {
                Ok(
                    LegacyTargetRecord::new("current", row.id, json!({"current":true}))
                        .source_metadata(row.meta),
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()
    })
    .unwrap();

    assert_eq!(converted.len(), 1);
    assert_eq!(converted[0].collection, "current");
    assert_eq!(converted[0].schema, Some(7));
    assert_eq!(converted[0].created_ms, Some(100));
    assert_eq!(converted[0].updated_ms, Some(200));
    assert_eq!(converted[0].expires_ms, Some(900));
}
