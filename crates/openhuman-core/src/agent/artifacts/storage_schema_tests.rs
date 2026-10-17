use super::*;
use tinystoragedrivers::Meta;

fn source(id: &str, body: Value) -> SourceRecord {
    SourceRecord {
        id: id.to_string(),
        body,
        meta: Meta::unknown(),
    }
}

#[test]
fn a_card_loses_its_formatted_creation_time() {
    let out = records_v2(vec![source(
        "a1",
        json!({
            "id": "a1", "kind": "document", "title": "T", "path": "a1/t.md", "size_bytes": 3,
            "status": "ready", "created_at": "1970-01-01T00:00:01Z", "created_ms": 1000,
            "thread_id": "th",
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, RECORDS);
    assert_eq!(out[0].id, "a1");
    assert!(out[0].body.get("created_at").is_none());
    assert_eq!(out[0].body["created_ms"], 1000);
    assert_eq!(out[0].body["thread_id"], "th");
}

#[test]
fn arguments_are_unwrapped() {
    let out = args_v2(vec![
        source("a1", json!({ "args": { "prompt": "x" } })),
        source("a2", json!({ "args": [1, 2] })),
    ])
    .unwrap();
    assert_eq!(out[0].body, json!({ "prompt": "x" }));
    assert_eq!(out[1].body, json!({ "_args": [1, 2] }));
    assert_eq!(args_of_body(&out[0].body), json!({ "prompt": "x" }));
    assert_eq!(args_of_body(&out[1].body), json!([1, 2]));
}

#[test]
fn a_versioned_record_gets_snake_case_keys_and_ms_times() {
    let v1 = json!({
        "id": "r1", "title": "T", "kind": "document", "status": "ready",
        "createdAtMillis": 1, "updatedAtMillis": 2, "updated_ms": 2,
        "versions": [{
            "version": 1, "body": "b", "author": "agent", "authorId": "x",
            "createdAtMillis": 1,
            "published": { "url": "u", "publishedAtMillis": 3, "expiresAtMillis": 9, "public": true },
        }],
    });
    let out = versions_v2(vec![source("r1", v1.clone())]).unwrap();
    assert_eq!(out[0].collection, VERSIONS);
    assert_eq!(
        out[0].body,
        json!({
            "id": "r1", "title": "T", "kind": "document", "status": "ready",
            "created_ms": 1, "updated_ms": 2,
            "versions": [{
                "version": 1, "body": "b", "author": "agent", "author_id": "x",
                "created_ms": 1,
                "published": { "url": "u", "published_ms": 3, "expires_ms": 9, "public": true },
            }],
        })
    );
    // The same mapping reads it back as the library's camelCase record.
    let mut expected = v1;
    expected.as_object_mut().unwrap().remove("updated_ms");
    assert_eq!(version_record(&out[0].body), expected);
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
