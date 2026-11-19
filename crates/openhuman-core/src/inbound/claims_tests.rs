//! Tests for the claims on events whose hand-off was ambiguous.

use super::*;

const NOW_MS: i64 = 1_000_000;

fn storage() -> crate::storage::Storage {
    tinystoragedrivers_testkit::memory_storage(&crate::storage::Scope::new("local").unwrap())
}

fn claims() -> Arc<Claims> {
    Arc::new(Claims::default())
}

#[test]
fn a_running_direct_path_holds_its_delivery_and_a_handled_one_drops_it() {
    let claims = claims();
    claims.open("composio:slack:e1");
    let claim = claims.take_direct("composio:slack:e1").unwrap();
    assert_eq!(claims.delivering("composio:slack:e1"), Delivery::Wait);
    claim.finish(true);
    assert_eq!(claims.delivering("composio:slack:e1"), Delivery::Skip);
    // And any redelivery of it.
    assert_eq!(claims.delivering("composio:slack:e1"), Delivery::Skip);
}

#[test]
fn a_direct_path_that_did_not_handle_its_event_releases_it_to_the_queues() {
    // A failed or deferred triage: the queued copy runs, nothing is lost.
    let claims = claims();
    claims
        .take_direct("composio:slack:e2")
        .unwrap()
        .finish(false);
    assert_eq!(claims.delivering("composio:slack:e2"), Delivery::Run);
}

#[test]
fn a_direct_claim_dropped_unfinished_is_released() {
    // The direct task ended without saying (a panic, a dropped future).
    let claims = claims();
    drop(claims.take_direct("composio:slack:e3").unwrap());
    assert_eq!(claims.delivering("composio:slack:e3"), Delivery::Run);
}

#[test]
fn the_direct_path_is_taken_once_per_event() {
    // A second ambiguous offer of the event (a platform redelivery during the
    // same outage) is not given it, running or done.
    let claims = claims();
    let first = claims.take_direct("composio:slack:e4").unwrap();
    assert!(claims.take_direct("composio:slack:e4").is_none());
    first.finish(true);
    assert!(claims.take_direct("composio:slack:e4").is_none());
}

#[test]
fn an_event_delivered_while_its_offer_waited_is_never_taken_for_the_direct_path() {
    let claims = claims();
    claims.open("composio:slack:e5");
    assert_eq!(claims.delivering("composio:slack:e5"), Delivery::Run);
    assert!(claims.take_direct("composio:slack:e5").is_none());
    // Not a later offer of it either: the queues own it.
    claims.open("composio:slack:e5");
    claims.close("composio:slack:e5");
    assert!(claims.take_direct("composio:slack:e5").is_none());
}

#[test]
fn an_answered_offer_leaves_no_claim() {
    let claims = claims();
    claims.open("composio:slack:e6");
    claims.close("composio:slack:e6");
    assert_eq!(claims.delivering("composio:slack:e6"), Delivery::Run);
    // Closing never undoes a direct-path claim.
    let claim = claims.take_direct("composio:slack:e7").unwrap();
    claims.close("composio:slack:e7");
    assert_eq!(claims.delivering("composio:slack:e7"), Delivery::Wait);
    claim.finish(true);
}

#[test]
fn an_event_never_offered_through_a_claim_is_delivered() {
    assert_eq!(claims().delivering("webhook:t1:c1"), Delivery::Run);
}

#[test]
fn a_handled_direct_path_event_is_skipped_after_a_restart() {
    let storage = storage();
    let before = Arc::new(Claims::over(&storage, NOW_MS));
    before.take_direct("webhook:t1:c2").unwrap().finish(true);
    // A running one is not recorded: after a restart its copy runs.
    let running = before.take_direct("webhook:t1:c3").unwrap();
    // A new process over the same storage.
    let after = Claims::over(&storage, NOW_MS + 1_000);
    assert_eq!(after.delivering("webhook:t1:c2"), Delivery::Skip);
    assert_eq!(after.delivering("webhook:t1:c3"), Delivery::Run);
    drop(running);
}

#[test]
fn a_direct_path_record_expires_with_the_module_tombstones() {
    let storage = storage();
    Arc::new(Claims::over(&storage, NOW_MS))
        .take_direct("webhook:t1:c4")
        .unwrap()
        .finish(true);
    // The record is stamped with the clock, so read it a TTL later.
    let now = chrono::Utc::now().timestamp_millis();
    let later = Claims::over(&storage, now + TTL_MS + 1);
    assert_eq!(later.delivering("webhook:t1:c4"), Delivery::Run);
    // The expired record was deleted, not just ignored.
    let again = Claims::over(&storage, now);
    assert_eq!(again.delivering("webhook:t1:c4"), Delivery::Run);
}

#[test]
fn a_handled_claim_evicted_from_memory_is_still_found_in_the_store() {
    let storage = storage();
    let claims = Arc::new(Claims::over(&storage, NOW_MS));
    claims
        .take_direct("composio:slack:old")
        .unwrap()
        .finish(true);
    for n in 0..CAPACITY {
        claims.open(&format!("composio:slack:b{n}"));
        claims.delivering(&format!("composio:slack:b{n}"));
    }
    assert!(!claims.memory().claims.contains_key("composio:slack:old"));
    assert_eq!(claims.delivering("composio:slack:old"), Delivery::Skip);
}

#[test]
fn settled_claims_are_bounded_in_memory() {
    let claims = claims();
    for n in 0..=CAPACITY {
        claims.open(&format!("composio:slack:b{n}"));
        claims.delivering(&format!("composio:slack:b{n}"));
    }
    let memory = claims.memory();
    assert_eq!(memory.claims.len(), CAPACITY);
    assert!(!memory.claims.contains_key("composio:slack:b0"));
}

#[test]
fn each_workspace_keeps_its_own_claims() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    Claims::of_workspace(a.path(), NOW_MS)
        .take_direct("composio:slack:w1")
        .unwrap()
        .finish(true);
    assert_eq!(
        Claims::of_workspace(b.path(), NOW_MS).delivering("composio:slack:w1"),
        Delivery::Run
    );
    assert_eq!(
        Claims::of_workspace(a.path(), NOW_MS).delivering("composio:slack:w1"),
        Delivery::Skip
    );
}

#[test]
fn a_turn_waits_for_a_running_direct_path_and_drops_only_handled_events() {
    use tinyeventqueues_bus::{InboundEvent, Lane};
    let event = |id: &str| {
        let envelope = crate::triage::TriggerEnvelope::from_composio(
            "slack",
            "SLACK_MESSAGE",
            "",
            id,
            serde_json::json!({}),
        );
        super::super::events::composio(&envelope).unwrap()
    };
    let turn = |events: Vec<InboundEvent>| Turn::new("t", Lane::Background, "c", events);
    let claims = claims();
    let running = claims.take_direct(&event("s1").dedupe_key()).unwrap();
    assert!(matches!(
        claims.sort(turn(vec![event("s1"), event("s2")])),
        Sorted::Wait(_)
    ));
    running.finish(true);
    match claims.sort(turn(vec![event("s1"), event("s2")])) {
        Sorted::Deliver(left) => {
            assert_eq!(left.events.len(), 1);
            assert_eq!(left.events[0].event_id, event("s2").event_id);
        }
        other => panic!("expected the rest delivered, got {other:?}"),
    }
    assert!(matches!(
        claims.sort(turn(vec![event("s1")])),
        Sorted::Handled
    ));
    assert!(matches!(claims.sort(turn(Vec::new())), Sorted::Deliver(_)));
}

#[test]
fn an_evicted_handled_claim_is_not_taken_for_the_direct_path_again() {
    let storage = storage();
    let claims = Arc::new(Claims::over(&storage, NOW_MS));
    claims
        .take_direct("composio:slack:gone")
        .unwrap()
        .finish(true);
    for n in 0..CAPACITY {
        claims.open(&format!("composio:slack:f{n}"));
        claims.delivering(&format!("composio:slack:f{n}"));
    }
    assert!(!claims.memory().claims.contains_key("composio:slack:gone"));
    // A repeated ambiguous offer of it, after eviction.
    claims.open("composio:slack:gone");
    assert!(claims.take_direct("composio:slack:gone").is_none());
}

#[test]
fn an_unsent_offer_does_not_release_another_pending_offer() {
    // Offer A is still waiting; offer B of the same event was never sent.
    let claims = claims();
    claims.open("composio:slack:p1");
    claims.open("composio:slack:p1");
    assert!(!claims.close("composio:slack:p1"), "A is still pending");
    // B goes through the claims, so A cannot take the direct path too.
    let b = claims.take_direct("composio:slack:p1").unwrap();
    assert!(claims.take_direct("composio:slack:p1").is_none());
    b.finish(true);
    // Alone, an unsent offer needs no claim.
    claims.open("composio:slack:p2");
    assert!(claims.close("composio:slack:p2"));
}

#[test]
fn a_handled_claim_the_store_could_not_take_is_written_once_it_reads() {
    let workspace = tempfile::tempdir().unwrap();
    let unread = || {
        Store::Unread(
            workspace.path().to_path_buf(),
            crate::storage::Scope::local(),
        )
    };
    let claims = Claims::with_store(unread());
    // Handled while the store was unreadable: held in memory, unevictable.
    claims.memory().hold("webhook:t1:held", NOW_MS);
    claims.read(NOW_MS);
    assert!(claims.memory().unrecorded.is_empty());
    // A new process over the same workspace finds it recorded.
    let after = Claims::with_store(unread());
    after.read(NOW_MS + 1_000);
    assert_eq!(after.delivering("webhook:t1:held"), Delivery::Skip);
}

#[test]
fn claims_without_a_storage_scope_are_shared_with_no_one() {
    let workspace = tempfile::tempdir().unwrap();
    let no_scope = || Err(crate::storage::StorageError::invalid_input("no profile"));
    let first = Claims::of_scope(workspace.path(), no_scope(), NOW_MS);
    first.take_direct("composio:slack:ns").unwrap().finish(true);
    let second = Claims::of_scope(workspace.path(), no_scope(), NOW_MS);
    assert_eq!(second.delivering("composio:slack:ns"), Delivery::Run);
}

#[test]
fn an_evicted_handled_event_offered_again_is_still_dropped_on_delivery() {
    let storage = storage();
    let claims = Arc::new(Claims::over(&storage, NOW_MS));
    claims
        .take_direct("composio:slack:again")
        .unwrap()
        .finish(true);
    for n in 0..CAPACITY {
        claims.open(&format!("composio:slack:g{n}"));
        claims.delivering(&format!("composio:slack:g{n}"));
    }
    claims.open("composio:slack:again");
    assert_eq!(claims.delivering("composio:slack:again"), Delivery::Skip);
}
