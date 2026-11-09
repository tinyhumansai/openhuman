#![cfg(feature = "storage-sqlite")]

use super::*;
use crate::storage::lease::{DocumentLeases, LeaseError, LeaseGrant, LeaseRecord};
use crate::storage::test_support::SqliteBackend;
use std::time::Duration;

const TTL: Duration = Duration::from_secs(30);

fn leases(backend: &SqliteBackend, node: &str) -> Arc<dyn LeaseStore> {
    Arc::new(DocumentLeases::cluster(backend.backend.as_ref(), node, None, TTL).unwrap())
}

fn scopes() -> Vec<ScopeMatch> {
    vec![
        ScopeMatch::Exact("profile:alice".into()),
        ScopeMatch::Prefix("alice~".into()),
    ]
}

fn fence(store: Arc<dyn LeaseStore>, node: &str, grant: &LeaseGrant, margin: u64) -> LeaseFence {
    LeaseFence::new(
        store,
        "alice",
        node,
        grant.epoch,
        grant.expires_at_ms,
        margin,
        scopes(),
    )
}

#[test]
fn a_fence_covers_its_exact_scope_and_prefix_only() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let grant = LeaseGrant {
        key: "alice".into(),
        epoch: 1,
        version: tinystoragedrivers_types::Version(1),
        expires_at_ms: 1_000,
        previous_unclean: false,
    };
    let f = fence(leases(&backend, "a"), "a", &grant, 0);
    assert!(f.covers("profile:alice"));
    assert!(f.covers("alice~default"));
    assert!(f.covers("alice~sub-1"));
    assert!(!f.covers("profile:alice2"));
    assert!(!f.covers("alice2~default"));
    assert!(!f.covers("cluster"));
}

#[test]
fn the_local_check_refuses_inside_the_skew_margin_and_after_renewal_admits_again() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let grant = LeaseGrant {
        key: "alice".into(),
        epoch: 3,
        version: tinystoragedrivers_types::Version(1),
        expires_at_ms: 30_000,
        previous_unclean: false,
    };
    let f = fence(leases(&backend, "a"), "a", &grant, 5_000);
    assert_eq!(f.check_local(24_999), Ok(()));
    assert_eq!(f.check_local(25_000), Err(FenceError::Expired { epoch: 3 }));
    f.renewed(60_000);
    assert_eq!(f.check_local(25_000), Ok(()));
    f.fence();
    assert_eq!(f.check_local(0), Err(FenceError::Fenced { epoch: 3 }));
}

#[tokio::test]
async fn the_full_check_follows_the_stored_record_and_latches_on_takeover() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let a = leases(&backend, "node-a");
    let grant = a.acquire("alice", 0).await.unwrap();
    let f = fence(Arc::clone(&a), "node-a", &grant, 0);
    assert_eq!(f.check(1_000).await, Ok(()));

    // node-b takes the key over once it expired by node-b's clock; node-a's
    // clock is behind (paused, or skewed), so only the record shows it.
    let b = leases(&backend, "node-b");
    let stolen = b.acquire("alice", 31_000).await.unwrap();
    assert_eq!(stolen.epoch, grant.epoch + 1);
    assert_eq!(
        f.check(1_000).await,
        Err(FenceError::Superseded {
            held: grant.epoch,
            stored: Some(stolen.epoch),
        })
    );
    assert!(f.is_fenced(), "a superseded fence latches");
    // `fenced()` resolves at once on a latched fence.
    tokio::time::timeout(Duration::from_secs(1), f.fenced())
        .await
        .expect("fenced() resolves");
}

#[tokio::test]
async fn a_released_or_missing_record_supersedes_the_fence() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let a = leases(&backend, "node-a");
    let grant = a.acquire("alice", 0).await.unwrap();
    let f = fence(Arc::clone(&a), "node-a", &grant, 0);
    a.release(grant.clone()).await.unwrap();
    assert!(matches!(
        f.check(1).await,
        Err(FenceError::Superseded {
            stored: Some(1),
            ..
        })
    ));

    let other = LeaseFence::new(Arc::clone(&a), "nobody", "node-a", 1, u64::MAX, 0, scopes());
    assert_eq!(
        other.check(1).await,
        Err(FenceError::Superseded {
            held: 1,
            stored: None
        })
    );

    // A fence never keeps its node's lease store alive; once the store is
    // gone, the check fails closed.
    let orphan = LeaseFence::new(a, "alice", "node-a", 2, u64::MAX, 0, scopes());
    assert!(matches!(
        orphan.check(1).await,
        Err(FenceError::Unverified(_))
    ));
}

#[derive(Debug)]
struct Unreachable;

#[async_trait::async_trait]
impl LeaseStore for Unreachable {
    async fn acquire(&self, _: &str, _: u64) -> Result<LeaseGrant, LeaseError> {
        Err(LeaseError::Storage(StorageError::unavailable("down")))
    }
    async fn renew(&self, _: &LeaseGrant, _: u64) -> Result<LeaseGrant, LeaseError> {
        Err(LeaseError::Storage(StorageError::unavailable("down")))
    }
    async fn release(&self, _: LeaseGrant) -> Result<(), LeaseError> {
        Err(LeaseError::Storage(StorageError::unavailable("down")))
    }
    async fn holder(&self, _: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        Err(LeaseError::Storage(StorageError::unavailable("down")))
    }
}

#[tokio::test]
async fn an_unreadable_record_refuses_the_write_without_latching() {
    let f = LeaseFence::new(
        Arc::new(Unreachable),
        "alice",
        "a",
        1,
        u64::MAX,
        0,
        scopes(),
    );
    assert!(matches!(f.check(1).await, Err(FenceError::Unverified(_))));
    assert!(!f.is_fenced(), "a storage hiccup is not a lost lease");
}

#[test]
fn a_refusal_survives_the_trip_through_a_port_error() {
    let error = FenceError::Expired { epoch: 7 }.into_storage();
    assert_eq!(error.kind(), tinystoragedrivers::ErrorKind::Backend);
    assert!(!error.is_retryable());
    assert_eq!(fence_error(&error), Some(&FenceError::Expired { epoch: 7 }));
    assert_eq!(fence_error(&StorageError::conflict("x")), None);
}

#[test]
fn the_registry_replaces_retires_and_prunes_dead_tombstones() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let store = leases(&backend, "a");
    let registry = FenceRegistry::new(|| 0);
    let make = |epoch| {
        Arc::new(LeaseFence::new(
            Arc::clone(&store),
            "alice",
            "a",
            epoch,
            u64::MAX,
            0,
            scopes(),
        ))
    };
    assert!(registry.lookup("profile:alice").is_none());

    let first = make(1);
    registry.register(Arc::clone(&first));
    assert_eq!(registry.lookup("alice~default").unwrap().epoch(), 1);

    // A newer grant on the same key replaces it.
    let second = make(2);
    registry.register(Arc::clone(&second));
    assert_eq!(registry.lookup("profile:alice").unwrap().epoch(), 2);

    // An older grant's fence never displaces a newer one.
    registry.register(Arc::clone(&first));
    assert_eq!(registry.lookup("profile:alice").unwrap().epoch(), 2);

    // Retiring a stale epoch leaves the current fence.
    registry.retire("alice", 1);
    assert_eq!(registry.lookup("profile:alice").unwrap().epoch(), 2);
    registry.retire("alice", 2);
    assert!(registry.lookup("profile:alice").is_none());

    // A latched fence stays while its anchor lives, then is pruned.
    let third = make(3);
    let anchor: Arc<dyn Any + Send + Sync> = Arc::new(());
    third.anchor_to(Arc::downgrade(&anchor));
    registry.register(Arc::clone(&third));
    third.fence();
    assert!(registry.lookup("profile:alice").unwrap().is_fenced());
    drop(anchor);
    assert!(registry.lookup("profile:alice").is_none());
}

#[test]
fn registration_prunes_a_dead_anchored_fence_before_comparing_epochs() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let store = leases(&backend, "a");
    let registry = FenceRegistry::new(|| 0);
    let make = |epoch| {
        Arc::new(LeaseFence::new(
            Arc::clone(&store),
            "alice",
            "a",
            epoch,
            u64::MAX,
            0,
            scopes(),
        ))
    };
    let dead_anchor_fence = make(2);
    let dead_anchor: Arc<dyn Any + Send + Sync> = Arc::new(());
    dead_anchor_fence.anchor_to(Arc::downgrade(&dead_anchor));
    registry.register(dead_anchor_fence);
    drop(dead_anchor);

    let lower_epoch = make(1);
    registry.register(Arc::clone(&lower_epoch));
    assert_eq!(registry.lookup("profile:alice").unwrap().epoch(), 1);
}

#[test]
fn registration_preserves_a_live_or_unanchored_higher_epoch_fence() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let store = leases(&backend, "a");
    let registry = FenceRegistry::new(|| 0);
    let make = |epoch| {
        Arc::new(LeaseFence::new(
            Arc::clone(&store),
            "alice",
            "a",
            epoch,
            u64::MAX,
            0,
            scopes(),
        ))
    };
    let higher = make(2);
    let live_anchor: Arc<dyn Any + Send + Sync> = Arc::new(());
    higher.anchor_to(Arc::downgrade(&live_anchor));
    registry.register(Arc::clone(&higher));
    registry.register(make(1));
    assert_eq!(registry.lookup("profile:alice").unwrap().epoch(), 2);

    // No anchor means ownership has not been tied to a disposable host yet;
    // it must remain protected from a lower-epoch registration.
    registry.retire("alice", 2);
    let unanchored = make(3);
    registry.register(Arc::clone(&unanchored));
    registry.register(make(1));
    assert_eq!(registry.lookup("profile:alice").unwrap().epoch(), 3);
}

#[tokio::test]
async fn the_guard_passes_unfenced_scopes_and_refuses_with_a_typed_error() {
    let backend = crate::storage::test_support::sqlite_backend().unwrap();
    let store = leases(&backend, "a");
    let grant = store.acquire("alice", 0).await.unwrap();
    let registry = FenceRegistry::new(|| 40_000);
    registry.register(Arc::new(fence(store, "a", &grant, 0)));
    registry.guard("profile:bob", "test").await.unwrap();
    let refused = registry.guard("profile:alice", "test").await.unwrap_err();
    assert_eq!(
        fence_error(&refused),
        Some(&FenceError::Expired { epoch: grant.epoch })
    );
}
