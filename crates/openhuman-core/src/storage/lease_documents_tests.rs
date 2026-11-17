#![cfg(feature = "storage-sqlite")]

use super::*;

use std::time::Duration;

use crate::storage::test_support::memory_backend;
use crate::storage::test_support::SqliteBackend;
use tinystoragedrivers_types::{CollectionSpec, Precondition, Scope};

const TTL: Duration = Duration::from_millis(1_000);

fn node(storage: &SqliteBackend, id: &str) -> DocumentLeases {
    DocumentLeases::cluster(
        storage.backend.as_ref(),
        id,
        Some(format!("http://{id}")),
        TTL,
    )
    .unwrap()
}

#[tokio::test]
async fn doctor_holder_inspection_does_not_declare_the_lease_collection() {
    let storage = memory_backend();
    let leases = DocumentLeases::cluster(storage.as_ref(), "a", None, TTL).unwrap();
    let documents = storage
        .for_scope(&Scope::new(CLUSTER_SCOPE).unwrap())
        .unwrap();
    let spec = CollectionSpec::new(LEASE_COLLECTION);

    assert!(documents
        .documents()
        .validate_collection_spec(&spec)
        .await
        .is_err());
    assert!(leases.holder_readonly("alice").await.unwrap().is_none());
    assert!(
        documents
            .documents()
            .validate_collection_spec(&spec)
            .await
            .is_err(),
        "a doctor-style holder inspection must not create storage schema"
    );
}

#[tokio::test]
async fn acquire_contend_expire_and_take_over() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let (a, b) = (node(&storage, "a"), node(&storage, "b"));

    let first = a.acquire("user-1", 0).await.unwrap();
    assert_eq!((first.epoch, first.previous_unclean), (1, false));
    assert_eq!(first.expires_at_ms, 1_000);

    match b.acquire("user-1", 500).await {
        Err(LeaseError::Held(record)) => {
            assert_eq!(record.owner, "a");
            assert_eq!(record.endpoint.as_deref(), Some("http://a"));
            assert_eq!(record.retry_after_ms(500), 500);
        }
        other => panic!("expected Held, got {other:?}"),
    }

    let taken = b.acquire("user-1", 1_000).await.unwrap();
    assert_eq!((taken.epoch, taken.previous_unclean), (2, true));
    assert_eq!(b.holder("user-1").await.unwrap().unwrap().owner, "b");

    assert!(matches!(
        a.renew(&first, 1_001).await,
        Err(LeaseError::Lost)
    ));
    assert!(matches!(a.release(first).await, Err(LeaseError::Lost)));
    assert_eq!(
        b.holder("user-1").await.unwrap().unwrap().owner,
        "b",
        "a stale release changes nothing"
    );
}

#[tokio::test]
async fn renew_extends_and_keeps_the_epoch() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let (a, b) = (node(&storage, "a"), node(&storage, "b"));
    let grant = a.acquire("k", 0).await.unwrap();
    let renewed = a.renew(&grant, 900).await.unwrap();
    assert_eq!(renewed.epoch, grant.epoch);
    assert_eq!(renewed.expires_at_ms, 1_900);
    assert!(renewed.version > grant.version);
    assert!(matches!(
        b.acquire("k", 1_500).await,
        Err(LeaseError::Held(_))
    ));
    assert!(
        matches!(a.renew(&grant, 950).await, Err(LeaseError::Lost)),
        "the superseded grant cannot renew"
    );
}

#[tokio::test]
async fn a_clean_release_hands_over_cleanly() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let (a, b) = (node(&storage, "a"), node(&storage, "b"));
    let grant = a.acquire("k", 0).await.unwrap();
    a.release(grant).await.unwrap();
    let record = a.holder("k").await.unwrap().unwrap();
    assert!(record.released && !record.is_live(1));
    let next = b.acquire("k", 1).await.unwrap();
    assert_eq!((next.epoch, next.previous_unclean), (2, false));
}

#[tokio::test]
async fn reacquire_is_reentrant_and_a_restart_is_unclean() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let a = node(&storage, "a");
    let grant = a.acquire("k", 0).await.unwrap();
    let again = a.acquire("k", 10).await.unwrap();
    assert_eq!((again.epoch, again.previous_unclean), (grant.epoch, false));
    assert!(matches!(a.renew(&grant, 20).await, Err(LeaseError::Lost)));

    let restarted = node(&storage, "a");
    let after = restarted.acquire("k", 20).await.unwrap();
    assert_eq!((after.epoch, after.previous_unclean), (2, true));
    assert!(matches!(a.renew(&again, 30).await, Err(LeaseError::Lost)));
}

#[tokio::test]
async fn bad_keys_and_corrupt_records_are_storage_errors() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let a = node(&storage, "a");
    assert!(matches!(
        a.acquire("../escape", 0).await,
        Err(LeaseError::Storage(_))
    ));
    assert!(matches!(a.holder("a/b").await, Err(LeaseError::Storage(_))));
    let scoped = storage
        .backend
        .for_scope(&Scope::new(CLUSTER_SCOPE).unwrap())
        .unwrap();
    scoped
        .documents()
        .put(
            LEASE_COLLECTION,
            tinystoragedrivers_types::Key::new(format!("{LEASE_OWNER}/k"))
                .unwrap()
                .as_str(),
            serde_json::json!({ "holder": 7 }),
            Precondition::None,
        )
        .await
        .unwrap();
    assert!(matches!(
        a.acquire("k", 0).await,
        Err(LeaseError::Storage(_))
    ));
}

#[tokio::test]
async fn keys_are_independent() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let (a, b) = (node(&storage, "a"), node(&storage, "b"));
    a.acquire("one", 0).await.unwrap();
    let other = b.acquire("two", 0).await.unwrap();
    assert_eq!(other.epoch, 1);
    assert!(a.holder("three").await.unwrap().is_none());
}

#[tokio::test]
async fn renew_after_expiry_is_lost_even_without_takeover() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let a = node(&storage, "a");
    let grant = a.acquire("k", 0).await.unwrap();
    assert!(matches!(
        a.renew(&grant, 2_000).await,
        Err(LeaseError::Lost)
    ));
    assert!(a.renew(&grant, 999).await.is_ok());
}

#[tokio::test]
async fn another_instances_grant_cannot_renew_or_release() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let (a, b) = (node(&storage, "a"), node(&storage, "b"));
    let grant = a.acquire("k", 0).await.unwrap();
    assert!(matches!(b.renew(&grant, 10).await, Err(LeaseError::Lost)));
    assert!(matches!(
        b.release(grant.clone()).await,
        Err(LeaseError::Lost)
    ));
    assert_eq!(a.holder("k").await.unwrap().unwrap().owner, "a");
    assert!(a.renew(&grant, 10).await.is_ok());
}

#[tokio::test]
async fn sub_millisecond_ttl_does_not_expire_at_acquisition() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let a = DocumentLeases::cluster(storage.backend.as_ref(), "a", None, Duration::ZERO).unwrap();
    let b = node(&storage, "b");
    let grant = a.acquire("k", 5).await.unwrap();
    assert!(grant.expires_at_ms > 5);
    assert!(matches!(b.acquire("k", 5).await, Err(LeaseError::Held(_))));
}

#[tokio::test]
async fn concurrent_renewals_of_one_grant_have_one_winner() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let a = std::sync::Arc::new(node(&storage, "a"));
    let grant = a.acquire("k", 0).await.unwrap();
    let go = || {
        let (a, grant) = (std::sync::Arc::clone(&a), grant.clone());
        tokio::spawn(async move { a.renew(&grant, 100).await.is_ok() })
    };
    let (x, y) = tokio::join!(go(), go());
    assert!(x.unwrap() ^ y.unwrap(), "exactly one renewal may succeed");
    // The heartbeat's next renewal, from the grant the winner returned, holds.
    assert!(a.holder("k").await.unwrap().unwrap().is_live(150));
}

#[tokio::test]
async fn doctor_inspection_refuses_a_legacy_leases_collection() {
    // Only the pre-rename `leases` collection exists: its records are not
    // read as "no holder", the doctor is told a migration is needed.
    let storage = memory_backend();
    let leases = DocumentLeases::cluster(storage.as_ref(), "d", None, TTL).unwrap();
    storage
        .for_scope(&Scope::new(CLUSTER_SCOPE).unwrap())
        .unwrap()
        .documents()
        .ensure_collection(&CollectionSpec::new("leases"))
        .await
        .unwrap();

    let error = leases.holder_readonly("alice").await.unwrap_err();
    assert!(
        error.to_string().contains("explicit storage migration"),
        "{error}"
    );
}

#[tokio::test]
async fn doctor_inspection_reports_a_live_foreign_holder() {
    let storage = crate::storage::test_support::sqlite_backend().unwrap();
    let grant = node(&storage, "a").acquire("alice", 0).await.unwrap();

    let record = node(&storage, "d")
        .holder_readonly("alice")
        .await
        .unwrap()
        .expect("the live holder's record");
    assert_eq!(record.owner, "a");
    assert_eq!(record.epoch, grant.epoch);
    assert!(record.is_live(500), "inside its TTL the holder is live");
    assert!(!record.is_live(1_000), "at its deadline it no longer is");
}
