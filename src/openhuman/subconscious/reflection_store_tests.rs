//! Lifecycle tests for `subconscious_reflections` + `subconscious_hotness_snapshots`.
//!
//! Builds an in-memory SQLite, runs the full subconscious DDL (so we
//! exercise the migration appended in `super::store::SCHEMA_DDL`), and
//! validates CRUD + idempotency + ordering.

use super::*;
use crate::openhuman::subconscious::reflection::{hydrate_draft, ReflectionDraft, ReflectionKind};
use rusqlite::Connection;

fn fresh_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-mem");
    // Run the same DDL that `with_connection` runs in production, so the
    // migration path is exercised.
    conn.execute_batch(crate::openhuman::subconscious::store::SCHEMA_DDL_FOR_TESTS)
        .expect("apply DDL");
    conn
}

fn sample_reflection(id: &str, created_at: f64) -> Reflection {
    let draft = ReflectionDraft {
        kind: ReflectionKind::HotnessSpike,
        body: format!("body for {id}"),
        proposed_action: Some("Take a look".into()),
        source_refs: vec!["entity:foo".into()],
    };
    hydrate_draft(draft, id.into(), created_at, Vec::new())
}

#[test]
fn add_and_get_round_trip() {
    let conn = fresh_conn();
    let r = sample_reflection("r1", 1.0);
    add_reflection(&conn, &r).expect("add");
    let got = get_reflection(&conn, "r1").expect("get").expect("present");
    assert_eq!(got, r);
}

#[test]
fn add_is_idempotent_on_id() {
    let conn = fresh_conn();
    let r = sample_reflection("dup", 5.0);
    add_reflection(&conn, &r).unwrap();
    let mut bumped = r.clone();
    bumped.body = "DIFFERENT — should not overwrite".into();
    add_reflection(&conn, &bumped).unwrap();
    let got = get_reflection(&conn, "dup").unwrap().unwrap();
    assert_eq!(got.body, "body for dup");
}

#[test]
fn list_recent_orders_newest_first() {
    let conn = fresh_conn();
    add_reflection(&conn, &sample_reflection("a", 1.0)).unwrap();
    add_reflection(&conn, &sample_reflection("b", 5.0)).unwrap();
    add_reflection(&conn, &sample_reflection("c", 3.0)).unwrap();
    let got = list_recent(&conn, 10, None).unwrap();
    let ids: Vec<&str> = got.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["b", "c", "a"]);
}

#[test]
fn list_recent_respects_since_ts() {
    let conn = fresh_conn();
    add_reflection(&conn, &sample_reflection("a", 1.0)).unwrap();
    add_reflection(&conn, &sample_reflection("b", 5.0)).unwrap();
    let got = list_recent(&conn, 10, Some(2.0)).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, "b");
}

#[test]
fn mark_acted_and_dismissed_set_timestamps() {
    let conn = fresh_conn();
    add_reflection(&conn, &sample_reflection("act", 1.0)).unwrap();
    add_reflection(&conn, &sample_reflection("dis", 1.0)).unwrap();
    mark_acted(&conn, "act", 50.0).unwrap();
    mark_dismissed(&conn, "dis", 60.0).unwrap();
    assert_eq!(
        get_reflection(&conn, "act").unwrap().unwrap().acted_on_at,
        Some(50.0)
    );
    assert_eq!(
        get_reflection(&conn, "dis").unwrap().unwrap().dismissed_at,
        Some(60.0)
    );
}

#[test]
fn hotness_snapshot_replace_clears_then_writes() {
    let mut conn = fresh_conn();
    replace_hotness_snapshots(&mut conn, &[("e1".into(), 0.5), ("e2".into(), 1.5)], 100.0).unwrap();
    let v1 = load_hotness_snapshots(&conn).unwrap();
    assert_eq!(v1.len(), 2);

    replace_hotness_snapshots(&mut conn, &[("e1".into(), 0.9)], 200.0).unwrap();
    let v2 = load_hotness_snapshots(&conn).unwrap();
    assert_eq!(v2.len(), 1);
    assert_eq!(v2[0], ("e1".to_string(), 0.9));
}

#[test]
fn hotness_snapshot_replace_with_empty_clears_table() {
    let mut conn = fresh_conn();
    replace_hotness_snapshots(&mut conn, &[("e1".into(), 0.1)], 1.0).unwrap();
    replace_hotness_snapshots(&mut conn, &[], 2.0).unwrap();
    assert!(load_hotness_snapshots(&conn).unwrap().is_empty());
}
