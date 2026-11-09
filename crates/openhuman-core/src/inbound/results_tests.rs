//! Tests for the store of pinned-agent results.

use super::*;
use crate::storage::Scope;
use tinystoragedrivers_testkit::memory_storage;

fn results() -> Results {
    let storage = memory_storage(&Scope::new("local").unwrap());
    Results::over(&storage)
}

fn running(turn_id: &str, source: &str, created_ms: u64) -> Stored {
    Stored::running(
        turn_id,
        source,
        "mentions",
        "teeny-mentions",
        vec!["e1".into(), "e2".into()],
        "the message",
        created_ms,
    )
}

#[test]
fn a_started_turn_is_running_until_it_finishes() {
    let results = results();
    let started = results.start(&running("t1", "x:mentions", 10)).unwrap();
    assert_eq!(started.result.status, ResultStatus::Running);
    // Starting again keeps the first record (a redelivery retries it).
    let again = results.start(&running("t1", "x:mentions", 99)).unwrap();
    assert_eq!(again.result.created_ms, 10);
    assert_eq!(results.running().unwrap().len(), 1);
    assert_eq!(results.get("t1").unwrap().unwrap().message, "the message");

    let done = results
        .finish("t1", Ok("draft".into()), 20)
        .unwrap()
        .unwrap();
    assert_eq!(done.result.status, ResultStatus::Completed);
    assert_eq!(done.result.text.as_deref(), Some("draft"));
    assert_eq!(done.result.completed_ms, Some(20));
    assert!(results.running().unwrap().is_empty());
    assert!(results
        .finish("nope", Ok(String::new()), 1)
        .unwrap()
        .is_none());

    results.start(&running("t2", "x:mentions", 11)).unwrap();
    let failed = results
        .finish("t2", Err("boom".into()), 30)
        .unwrap()
        .unwrap();
    assert_eq!(failed.result.status, ResultStatus::Failed);
    assert_eq!(failed.result.error.as_deref(), Some("boom"));
    assert_eq!(failed.result.text, None);
}

#[test]
fn results_list_oldest_first_by_source_status_and_time_without_the_acked() {
    let results = results();
    for (turn, source, at) in [
        ("t3", "x:mentions", 30),
        ("t1", "x:mentions", 10),
        ("t2", "x:dms", 20),
        ("t4", "x:mentions", 40),
    ] {
        results.start(&running(turn, source, at)).unwrap();
    }
    results.finish("t1", Ok("a".into()), 50).unwrap();
    let ids = |query: ResultQuery| -> Vec<String> {
        results
            .list(&query)
            .unwrap()
            .into_iter()
            .map(|r| r.turn_id)
            .collect()
    };
    assert_eq!(ids(ResultQuery::default()), ["t1", "t2", "t3", "t4"]);
    let mentions = ResultQuery {
        source: Some("x:mentions".into()),
        ..ResultQuery::default()
    };
    assert_eq!(ids(mentions.clone()), ["t1", "t3", "t4"]);
    assert_eq!(
        ids(ResultQuery {
            since_ms: Some(30),
            ..mentions.clone()
        }),
        ["t3", "t4"]
    );
    assert_eq!(
        ids(ResultQuery {
            status: Some(ResultStatus::Completed),
            ..ResultQuery::default()
        }),
        ["t1"]
    );
    assert_eq!(
        ids(ResultQuery {
            limit: 2,
            ..ResultQuery::default()
        }),
        ["t1", "t2"]
    );

    assert!(results.ack("t1").unwrap());
    assert!(!results.ack("missing").unwrap());
    assert_eq!(ids(ResultQuery::default()), ["t2", "t3", "t4"]);
    // The wire form leaves out the message and the ack flag.
    let wire = serde_json::to_value(&results.list(&ResultQuery::default()).unwrap()[0]).unwrap();
    assert!(wire.get("message").is_none() && wire.get("acked").is_none());
    assert_eq!(wire["status"], "running");
    assert_eq!(wire["event_ids"], serde_json::json!(["e1", "e2"]));
    assert_eq!(wire["blocked"], serde_json::json!([]));
}

#[test]
fn blocked_events_are_kept_with_the_result() {
    let results = results();
    let blocked = vec![BlockedEvent {
        event_id: "e2".into(),
        reason: "blocked: instruction_override".into(),
    }];
    results
        .start(&running("t1", "x:mentions", 1).with_blocked(blocked.clone()))
        .unwrap();
    let listed = results.list(&ResultQuery::default()).unwrap();
    assert_eq!(listed[0].blocked, blocked);
    assert_eq!(
        serde_json::to_value(&listed[0]).unwrap()["blocked"],
        serde_json::json!([{ "event_id": "e2", "reason": "blocked: instruction_override" }])
    );
}

#[test]
fn results_past_their_source_ttl_are_pruned() {
    let results = results();
    results.start(&running("old", "x:mentions", 1_000)).unwrap();
    results.start(&running("new", "x:mentions", 9_000)).unwrap();
    results.start(&running("dm", "x:dms", 1_000)).unwrap();
    let pruned = results
        .prune(10_000, |source| {
            if source == "x:mentions" {
                5_000
            } else {
                50_000
            }
        })
        .unwrap();
    assert_eq!(pruned, 1);
    assert!(results.get("old").unwrap().is_none());
    assert!(results.get("new").unwrap().is_some());
    assert!(results.get("dm").unwrap().is_some());
}
