use super::*;
use serde_json::json;
use tinystoragedrivers::Meta;

fn source(id: &str, body: Value) -> SourceRecord {
    SourceRecord {
        id: id.to_string(),
        body,
        meta: Meta::unknown(),
    }
}

#[test]
fn a_v1_row_becomes_epoch_millisecond_fields() {
    let out = paired_v2(vec![source(
        "ch-1",
        json!({
            "label": "iPhone",
            "device_pubkey": "pk",
            "core_session_token_hash": "h",
            "created_at": "1970-01-01T00:00:01.000+00:00",
            "last_seen_at": "1970-01-01T00:00:02.500Z",
            "revoked": true,
        }),
    )])
    .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].collection, DEVICES);
    assert_eq!(out[0].id, "ch-1");
    assert_eq!(
        out[0].body,
        json!({
            "label": "iPhone",
            "device_pubkey": "pk",
            "core_session_token_hash": "h",
            "created_ms": 1000,
            "last_seen_ms": 2500,
            "revoked": true,
        })
    );
}

#[test]
fn a_never_seen_device_has_no_last_seen_field() {
    let out = paired_v2(vec![source(
        "ch-2",
        json!({
            "label": "a", "device_pubkey": "p", "core_session_token_hash": "h",
            "created_at": "1970-01-01T00:00:01Z", "last_seen_at": null, "revoked": false,
        }),
    )])
    .unwrap();
    assert!(out[0].body.get("last_seen_ms").is_none());
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
