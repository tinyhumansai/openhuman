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
fn a_source_keeps_its_settings_and_drops_formatted_times() {
    let out = sources_v2(vec![source(
        "s1",
        json!({
            "provider": "linear", "enabled": true, "filter": "{}", "interval_secs": 300,
            "target": "{}", "max_tasks_per_fetch": 5, "name": "mine",
            "created_at": "1970-01-01T00:00:01Z", "created_ms": 1000,
            "last_fetch_at": "1970-01-01T00:00:02Z", "last_status": "ok",
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, SOURCES);
    assert_eq!(out[0].id, "s1");
    let body = &out[0].body;
    assert!(body.get("created_at").is_none());
    assert!(body.get("last_fetch_at").is_none());
    assert_eq!(body["created_ms"], 1000);
    assert_eq!(body["last_fetch_ms"], 2000);
    assert_eq!(body["interval_secs"], 300);
}

#[test]
fn a_source_from_the_pre_ms_shape_gets_its_creation_time_from_the_text() {
    let out = sources_v2(vec![source(
        "s2",
        json!({ "provider": "x", "created_at": "1970-01-01T00:00:05Z" }),
    )])
    .unwrap();
    assert_eq!(out[0].body["created_ms"], 5000);
    assert!(out[0].body.get("last_fetch_ms").is_none());
}

#[test]
fn a_ledger_entry_is_rekeyed_by_source_and_external_id() {
    let out = ingested_v2(vec![source(
        "3:a/b/c",
        json!({
            "source_id": "a/b", "external_id": "c", "content_hash": "h", "title": "t",
            "payload": "{}", "ingested_at": "1970-01-01T00:00:01Z",
            "ingested_ms": 1000, "ingested_ns": 1_000_000_000_i64,
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, INGESTED);
    assert_eq!(out[0].id, ingested_id("a/b", "c"));
    assert_eq!(out[0].id, "a%2Fb/c");
    assert_eq!(
        out[0].body,
        json!({
            "source_id": "a/b", "external_id": "c", "content_hash": "h", "title": "t",
            "payload": "{}", "ingested_ms": 1000,
        })
    );
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
