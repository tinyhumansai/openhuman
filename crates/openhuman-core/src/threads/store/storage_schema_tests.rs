use super::*;
use serde_json::{json, Value};
use tinystoragedrivers::Meta;

#[test]
fn a_recorded_key_moves_under_the_threads_owner_keeping_its_body() {
    let out = transcript_keys_v2(vec![SourceRecord {
        id: "thread/1".into(),
        body: json!({ "key": "profile~agent" }),
        meta: Meta::unknown(),
    }])
    .unwrap();
    assert_eq!(out[0].collection, TRANSCRIPT_KEYS);
    assert_eq!(out[0].id, "thread%2F1");
    assert_eq!(out[0].body, json!({ "key": "profile~agent" }) as Value);
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
