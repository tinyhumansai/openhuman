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
fn the_usage_timestamp_is_carried_once_as_milliseconds() {
    let out = records_v2(vec![source(
        "r1",
        json!({
            "id": "r1", "session_id": "s",
            "usage": { "model": "m", "timestamp": "1970-01-01T00:00:02Z", "cost_usd": 0.5 },
            "ts_ms": 2000,
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, RECORDS);
    assert_eq!(out[0].id, "r1");
    assert_eq!(
        out[0].body,
        json!({
            "id": "r1", "session_id": "s",
            "usage": { "model": "m", "cost_usd": 0.5 },
            "ts_ms": 2000,
        })
    );
}

#[test]
fn a_row_without_ts_ms_takes_it_from_the_timestamp() {
    let out = records_v2(vec![source(
        "r2",
        json!({ "id": "r2", "session_id": "s", "usage": { "timestamp": "1970-01-01T00:00:03Z" } }),
    )])
    .unwrap();
    assert_eq!(out[0].body["ts_ms"], 3000);
}

#[test]
fn a_record_round_trips_through_its_body() {
    let usage = super::super::types::TokenUsage::new("m", 10, 5, 1.0, 2.0);
    let record = CostRecord::new("s", usage);
    let body = to_body(&record).unwrap();
    assert!(body["usage"].get("timestamp").is_none());
    let back = from_body(body).unwrap();
    assert_eq!(back.id, record.id);
    assert_eq!(
        back.usage.timestamp.timestamp_millis(),
        record.usage.timestamp.timestamp_millis()
    );
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
