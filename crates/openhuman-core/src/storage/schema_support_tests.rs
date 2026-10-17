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
