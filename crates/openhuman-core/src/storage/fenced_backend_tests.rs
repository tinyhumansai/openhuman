//! The host checks local lease state on cached handles; native conditions
//! independently fence each storage port against cross-process takeover.

#![cfg(feature = "storage-sqlite")]

use super::*;
use crate::storage::fence::{fence_error, FenceError, LeaseFence, ScopeMatch};
use crate::storage::lease::{DocumentLeases, LeaseGrant, LeaseStore, LocalLeases};
use serde_json::json;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tinystoragedrivers_types::{CollectionSpec, NewMessage, Precondition, QueueSpec, Scope};

const TTL: Duration = Duration::from_secs(30);
const MARGIN_MS: u64 = 5_000;

struct Node {
    name: &'static str,
    now: Arc<AtomicU64>,
    leases: Arc<DocumentLeases>,
    fences: Arc<FenceRegistry>,
    backend: FencedBackend,
}

impl Node {
    fn new(name: &'static str, shared: Arc<dyn StorageBackend>) -> Self {
        let now = Arc::new(AtomicU64::new(crate::storage::fence::system_now_ms()));
        let clock = Arc::clone(&now);
        let fences = Arc::new(FenceRegistry::new(move || clock.load(Ordering::SeqCst)));
        let leases = Arc::new(DocumentLeases::cluster(shared.as_ref(), name, None, TTL).unwrap());
        Self {
            name,
            now,
            leases,
            fences: Arc::clone(&fences),
            backend: FencedBackend::new(shared, fences),
        }
    }

    fn set_now(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }

    async fn acquire(&self, key: &str) -> (LeaseGrant, Arc<LeaseFence>) {
        let grant = self
            .leases
            .acquire(key, self.now.load(Ordering::SeqCst))
            .await
            .unwrap();
        let fence = Arc::new(LeaseFence::new(
            Arc::clone(&self.leases) as Arc<dyn LeaseStore>,
            key,
            self.name,
            grant.epoch,
            grant.expires_at_ms,
            MARGIN_MS,
            vec![ScopeMatch::Exact(format!("profile:{key}"))],
        ));
        self.fences.register(Arc::clone(&fence));
        (grant, fence)
    }

    fn storage(&self) -> Storage {
        self.backend
            .for_scope(&Scope::new("profile:alice").unwrap())
            .unwrap()
    }
}

fn shared() -> crate::storage::test_support::SqliteBackend {
    crate::storage::test_support::sqlite_backend().unwrap()
}

#[tokio::test]
async fn a_file_lease_guards_profile_handles_until_release() {
    let shared = shared();
    let root = tempfile::tempdir().unwrap();
    let leases: Arc<dyn LeaseStore> = Arc::new(LocalLeases::new(root.path(), "node-a"));
    let grant = leases.acquire("alice", 0).await.unwrap();
    let fences = Arc::new(FenceRegistry::new(crate::storage::fence::system_now_ms));
    let fence = Arc::new(LeaseFence::new(
        Arc::clone(&leases),
        "alice",
        "node-a",
        grant.epoch,
        grant.expires_at_ms,
        0,
        vec![ScopeMatch::Exact("profile:alice".into())],
    ));
    fences.register(Arc::clone(&fence));
    let backend = FencedBackend::new(Arc::clone(&shared.backend), Arc::clone(&fences));
    let scope = Scope::new("profile:alice").unwrap();
    let storage = backend.for_scope(&scope).unwrap();
    storage
        .documents()
        .ensure_collection(&CollectionSpec::new("items"))
        .await
        .unwrap();
    storage
        .documents()
        .put(
            "items",
            "during-lease",
            json!({"ok": true}),
            Precondition::Absent,
        )
        .await
        .unwrap();

    fences.retire("alice", grant.epoch);
    leases.release(grant).await.unwrap();
    let error = storage
        .documents()
        .put(
            "items",
            "after-release",
            json!({"ok": false}),
            Precondition::Absent,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        fence_error(&error),
        Some(FenceError::Fenced { .. })
    ));
}

#[tokio::test]
async fn cached_handles_enforce_local_expiry_margin_and_accept_same_epoch_renewal() {
    let shared = shared();
    let node = Node::new("node-a", Arc::clone(&shared.backend));
    let base = node.now.load(Ordering::SeqCst);
    let (grant, fence) = node.acquire("alice").await;
    let storage = node.storage();
    storage
        .documents()
        .ensure_collection(&CollectionSpec::new("items"))
        .await
        .unwrap();

    node.set_now(base + 20_000);
    let renewed = node.leases.renew(&grant, base + 20_000).await.unwrap();
    assert_eq!(renewed.epoch, grant.epoch);
    fence.renewed(renewed.expires_at_ms);
    node.set_now(base + 40_000);
    storage
        .documents()
        .put(
            "items",
            "renewed",
            json!({"ok": true}),
            Precondition::Absent,
        )
        .await
        .expect("the bound condition reads the renewed deadline dynamically");

    node.set_now(renewed.expires_at_ms - MARGIN_MS);
    let error = storage
        .documents()
        .put("items", "too-late", json!({}), Precondition::Absent)
        .await
        .unwrap_err();
    assert!(matches!(
        fence_error(&error),
        Some(FenceError::Expired { .. })
    ));
    assert!(storage
        .documents()
        .get("items", "too-late")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_cached_handle_cannot_mutate_after_cross_process_takeover_on_any_port() {
    let shared = shared();
    let a = Node::new("node-a", Arc::clone(&shared.backend));
    let b = Node::new("node-b", Arc::clone(&shared.backend));
    let base = a.now.load(Ordering::SeqCst);
    let (_grant, _fence) = a.acquire("alice").await;
    let storage = a.storage();
    storage
        .documents()
        .ensure_collection(&CollectionSpec::new("items"))
        .await
        .unwrap();
    storage
        .documents()
        .put("items", "kept", json!({}), Precondition::Absent)
        .await
        .unwrap();
    storage
        .queues()
        .ensure_queue("work", &QueueSpec::default())
        .await
        .unwrap();
    storage
        .queues()
        .enqueue("work", &[NewMessage::new(json!({"job": 1}))])
        .await
        .unwrap();
    let delivery = storage
        .queues()
        .receive("work", 1, 10_000, "worker")
        .await
        .unwrap();
    let receipt = delivery[0].receipt.clone();

    b.set_now(base + TTL.as_millis() as u64 + 1);
    let (stolen, _) = b.acquire("alice").await;
    assert_eq!(stolen.epoch, 2);
    // A's local clock is deliberately behind. The local precheck succeeds;
    // the driver's atomic condition is what refuses each cached-handle write.
    a.set_now(base + 1_000);
    assert!(storage
        .documents()
        .put("items", "late", json!({}), Precondition::Absent)
        .await
        .is_err());
    assert!(storage.streams().append("events", json!(1)).await.is_err());
    assert!(storage
        .blobs()
        .put("blob", b"x".to_vec(), None)
        .await
        .is_err());
    assert!(storage.kv().put("key", json!(1)).await.is_err());
    let error = storage.queues().ack("work", &receipt).await.unwrap_err();
    assert!(matches!(
        fence_error(&error),
        Some(FenceError::Fenced { .. })
    ));

    let raw = shared
        .backend
        .for_scope(&Scope::new("profile:alice").unwrap())
        .unwrap();
    assert!(raw
        .documents()
        .get("items", "late")
        .await
        .unwrap()
        .is_none());
    assert_eq!(raw.streams().len("events").await.unwrap(), 0);
    assert!(raw.blobs().get("blob").await.unwrap().is_none());
    assert!(raw.kv().get("key").await.unwrap().is_none());
    assert_eq!(raw.queues().stats("work").await.unwrap().leased, 1);
    assert!(raw
        .documents()
        .get("items", "kept")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn a_manually_latched_cached_fence_refuses_mutations_before_driver_io() {
    let shared = shared();
    let node = Node::new("node-a", Arc::clone(&shared.backend));
    let (_grant, fence) = node.acquire("alice").await;
    let storage = node.storage();
    storage
        .documents()
        .ensure_collection(&CollectionSpec::new("items"))
        .await
        .unwrap();
    fence.fence();
    assert!(storage
        .documents()
        .put("items", "late", json!({}), Precondition::Absent)
        .await
        .is_err());
    assert!(shared
        .backend
        .for_scope(&Scope::new("profile:alice").unwrap())
        .unwrap()
        .documents()
        .get("items", "late")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn captured_profile_sweeps_cannot_follow_a_replacement_epoch() {
    let shared = shared();
    let a = Node::new("node-a", Arc::clone(&shared.backend));
    let b = Node::new("node-b", Arc::clone(&shared.backend));
    let base = a.now.load(Ordering::SeqCst);
    let (_grant, fence) = a.acquire("alice").await;
    let captured = FencedBackend::for_fence(
        Arc::clone(&shared.backend),
        Arc::clone(&a.fences),
        Arc::clone(&fence),
    );
    let scope = Scope::new("profile:alice").unwrap();
    let storage = captured.for_scope(&scope).unwrap();
    storage
        .blobs()
        .put_expiring("aging", b"preserve".to_vec(), None, Duration::ZERO)
        .await
        .unwrap();

    b.set_now(base + TTL.as_millis() as u64 + 1);
    b.acquire("alice").await;
    a.set_now(base + 1_000);
    assert_eq!(
        super::super::blob_sweep::sweep_scope(&captured, &scope).await,
        0
    );
}

#[path = "fenced_backend_upstream_tests.rs"]
mod upstream_tests;

#[tokio::test]
async fn fenced_handles_declare_collections_on_a_backend_that_refuses_conditioned_declarations() {
    // Leases kept in the backend put a driver condition on every profile
    // handle; MongoDB refuses to declare collections under one (#8422).
    let shared = shared();
    let refusing = crate::storage::test_support::refusing_conditioned_declarations(Arc::clone(
        &shared.backend,
    ));
    let node = Node::new("node-a", Arc::clone(&refusing));
    let (_grant, fence) = node.acquire("alice").await;
    assert!(
        fence.write_condition().is_some(),
        "a lease kept in the backend has a driver condition"
    );
    let storage = node.storage();
    let spec = CollectionSpec::new("declared_under_lease");
    storage.documents().ensure_collection(&spec).await.unwrap();
    storage
        .documents()
        .validate_collection_spec(&spec)
        .await
        .unwrap();
    storage
        .documents()
        .put(
            "declared_under_lease",
            "row",
            json!({}),
            Precondition::Absent,
        )
        .await
        .unwrap();

    // An import claim on top of the lease (an admitted profile's import),
    // taken through the fenced backend and released before it is used, so
    // its condition no longer holds.
    let profile = Scope::new("profile:alice").unwrap();
    let fenced: Arc<dyn StorageBackend> = Arc::new(FencedBackend::new(
        Arc::clone(&refusing),
        Arc::clone(&node.fences),
    ));
    let claims = crate::storage::documents::Repo::on(
        &fenced,
        &crate::storage::Scope::from(profile.clone()),
        "fence-claim-test",
        Vec::new,
    )
    .unwrap();
    let stale = tokio::task::spawn_blocking(move || {
        crate::storage::import_state::with_claim(&claims, "fence_test", "import", |condition| {
            Ok(condition.clone())
        })
    })
    .await
    .unwrap()
    .unwrap()
    .expect("the claim is free");

    let conditioned = node
        .backend
        .for_scope_conditioned(&profile, &stale)
        .unwrap();
    let docs = conditioned.documents();
    docs.ensure_collection(&CollectionSpec::new("declared_under_claim"))
        .await
        .expect("a lease and a lost claim may still declare a collection");
    let written = docs
        .put(
            "declared_under_claim",
            "stale",
            json!({}),
            Precondition::Absent,
        )
        .await;
    assert!(
        written.is_err(),
        "a lost claim cannot write under the lease"
    );
    assert!(storage
        .documents()
        .get("declared_under_claim", "stale")
        .await
        .unwrap()
        .is_none());
}
