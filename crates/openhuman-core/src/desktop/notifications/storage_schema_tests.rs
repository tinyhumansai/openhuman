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
fn an_integration_notification_loses_its_formatted_times() {
    let out = integration_v2(vec![source(
        "n1",
        json!({
            "provider": "slack", "title": "t", "body": "b", "raw_payload": "{}",
            "status": "unread", "account_id": "acct",
            "received_at": "1970-01-01T00:00:01Z", "received_ms": 1000,
            "importance_score": 0.5, "triage_action": "react", "triage_reason": "r",
            "scored_at": "1970-01-01T00:00:02Z",
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, NOTIFICATIONS);
    assert_eq!(out[0].id, "n1");
    let body = &out[0].body;
    assert!(body.get("received_at").is_none());
    assert!(body.get("scored_at").is_none());
    assert_eq!(body["received_ms"], 1000);
    assert_eq!(body["scored_ms"], 2000);
    assert_eq!(body["importance_score"], 0.5);
}

#[test]
fn an_unscored_notification_has_no_scored_field() {
    let out = integration_v2(vec![source(
        "n2",
        json!({
            "provider": "slack", "title": "t", "body": "b", "raw_payload": "{}",
            "status": "unread", "received_ms": 7,
        }),
    )])
    .unwrap();
    assert!(out[0].body.get("scored_ms").is_none());
    assert!(out[0].body.get("importance_score").is_none());
}

#[test]
fn a_legacy_core_id_is_split_into_workspace_and_event() {
    assert_eq!(split_legacy_core_id("5:local/e1"), Some(("local", "e1")));
    assert_eq!(
        split_legacy_core_id("9:a/b:c/d/ev"),
        None,
        "a wrong length is not a v1 id"
    );
    assert_eq!(split_legacy_core_id("junk"), None);
}

#[test]
fn a_core_notification_is_rekeyed_by_workspace_and_event() {
    let out = core_v2(vec![source(
        "5:local/e/1",
        json!({
            "workspace": "local", "payload": "{}", "timestamp_ms": 9, "read": true,
            "created_at": "1970-01-01T00:00:03Z",
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, CORE);
    assert_eq!(out[0].id, core_id("local", "e/1"));
    assert_eq!(out[0].id, "local/e%2F1");
    assert_eq!(out[0].created_ms, Some(3000));
    assert_eq!(
        out[0].body,
        json!({ "workspace": "local", "payload": "{}", "timestamp_ms": 9, "read": true })
    );
}

#[test]
fn dedup_and_settings_keep_their_bodies() {
    let out = renamed(DEDUP)(vec![source("abc", json!({ "last_ms": 5 }))]).unwrap();
    assert_eq!(out[0].collection, DEDUP);
    assert_eq!(out[0].body, json!({ "last_ms": 5 }));
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
