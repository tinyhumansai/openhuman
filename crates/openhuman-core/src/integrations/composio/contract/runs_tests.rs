//! Tests for the sync-run report vocabulary — the pure-data half.
//!
//! What is pinned here is what two separately compiled processes have to agree
//! on: the `snake_case` reason tags and the reply shape.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::{SyncOutcome, SyncReason};

#[test]
fn every_sync_reason_tag_matches_its_serde_form() {
    for reason in [
        SyncReason::ConnectionCreated,
        SyncReason::Periodic,
        SyncReason::Manual,
    ] {
        let json = serde_json::to_string(&reason).expect("serialize");
        assert_eq!(
            json,
            format!("\"{}\"", reason.as_str()),
            "as_str and the serde form disagree for {reason:?}"
        );
        let back: SyncReason = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, reason);
    }
}

#[test]
fn sync_reason_tags_are_the_stable_strings() {
    assert_eq!(SyncReason::ConnectionCreated.as_str(), "connection_created");
    assert_eq!(SyncReason::Periodic.as_str(), "periodic");
    assert_eq!(SyncReason::Manual.as_str(), "manual");
}

#[test]
fn an_outcome_round_trips_with_its_open_details_object() {
    let outcome = SyncOutcome {
        toolkit: "gmail".into(),
        connection_id: Some("conn-1".into()),
        reason: SyncReason::Periodic.as_str().to_string(),
        items_ingested: 12,
        started_at_ms: 5,
        finished_at_ms: 9,
        summary: "12 messages".into(),
        details: serde_json::json!({ "pages": 3 }),
    };
    let json = serde_json::to_string(&outcome).expect("serialize");
    let back: SyncOutcome = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back.toolkit, "gmail");
    assert_eq!(back.connection_id.as_deref(), Some("conn-1"));
    assert_eq!(back.reason, "periodic");
    assert_eq!(back.items_ingested, 12);
    assert_eq!(back.summary, "12 messages");
    assert_eq!(back.details["pages"], 3);
}

#[test]
fn an_outcome_decodes_when_details_is_absent() {
    // `details` is `#[serde(default)]`, so an older peer that never wrote the
    // field still decodes rather than failing the whole frame.
    let back: SyncOutcome = serde_json::from_str(
        r#"{"toolkit":"slack","connection_id":null,"reason":"manual",
            "items_ingested":0,"started_at_ms":0,"finished_at_ms":0,"summary":""}"#,
    )
    .expect("deserialize without details");
    assert!(back.details.is_null());
}
