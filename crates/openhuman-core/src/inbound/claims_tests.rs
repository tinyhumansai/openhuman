//! Tests for the claims on events whose hand-off was ambiguous.

use super::*;

const NOW_MS: i64 = 1_000_000;

fn storage() -> crate::storage::Storage {
    tinystoragedrivers_testkit::memory_storage(&crate::storage::Scope::new("local").unwrap())
}

#[test]
fn an_event_taken_for_the_direct_path_is_skipped_by_its_delivery() {
    let claims = Claims::default();
    claims.open("composio:slack:e1");
    assert!(claims.take_direct("composio:slack:e1", NOW_MS));
    assert!(claims.delivering("composio:slack:e1"));
    // And by any redelivery of it.
    assert!(claims.delivering("composio:slack:e1"));
}

#[test]
fn an_event_delivered_while_its_offer_waited_is_not_taken_for_the_direct_path() {
    let claims = Claims::default();
    claims.open("composio:slack:e2");
    assert!(!claims.delivering("composio:slack:e2"));
    assert!(!claims.take_direct("composio:slack:e2", NOW_MS));
    // Settled: nothing is left behind.
    assert!(!claims.delivering("composio:slack:e2"));
}

#[test]
fn an_answered_offer_leaves_no_claim() {
    let claims = Claims::default();
    claims.open("composio:slack:e3");
    claims.close("composio:slack:e3");
    assert!(!claims.delivering("composio:slack:e3"));
    // Closing never undoes a direct-path claim.
    assert!(claims.take_direct("composio:slack:e4", NOW_MS));
    claims.close("composio:slack:e4");
    assert!(claims.delivering("composio:slack:e4"));
}

#[test]
fn an_event_never_offered_through_a_claim_is_delivered() {
    let claims = Claims::default();
    assert!(!claims.delivering("webhook:t1:c1"));
}

#[test]
fn a_direct_path_event_is_skipped_after_a_restart() {
    let storage = storage();
    let before = Claims::over(&storage, NOW_MS);
    assert!(before.take_direct("webhook:t1:c2", NOW_MS));
    // A new process over the same storage.
    let after = Claims::over(&storage, NOW_MS + 1_000);
    assert!(after.delivering("webhook:t1:c2"));
    assert!(!after.delivering("webhook:t1:c3"));
}

#[test]
fn a_direct_path_record_expires_with_the_module_tombstones() {
    let storage = storage();
    assert!(Claims::over(&storage, NOW_MS).take_direct("webhook:t1:c4", NOW_MS));
    let later = Claims::over(&storage, NOW_MS + TTL_MS + 1);
    assert!(!later.delivering("webhook:t1:c4"));
    // The expired record was deleted, not just ignored.
    let again = Claims::over(&storage, NOW_MS);
    assert!(!again.delivering("webhook:t1:c4"));
}

#[test]
fn direct_path_claims_are_bounded_in_memory() {
    let claims = Claims::default();
    for n in 0..=CAPACITY {
        assert!(claims.take_direct(&format!("composio:slack:b{n}"), NOW_MS));
    }
    assert!(!claims.delivering("composio:slack:b0"));
    assert!(claims.delivering(&format!("composio:slack:b{CAPACITY}")));
}
