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
fn channel_attachment_and_viseme_records_are_declared_in_the_threads_owner() {
    let names: Vec<String> = collection_specs()
        .iter()
        .map(|spec| spec.name.clone())
        .collect();
    for name in [
        TRANSCRIPT_KEYS,
        CHANNEL_MESSAGES,
        STAGED_ATTACHMENTS,
        REPLY_VISEME_TIMELINES,
    ] {
        assert!(
            names.iter().any(|declared| declared == name),
            "missing {name} collection: {names:?}"
        );
    }
}

#[test]
fn the_schema_composes_under_its_owner() {
    for spec in collection_specs() {
        spec.validate().unwrap();
    }
}
