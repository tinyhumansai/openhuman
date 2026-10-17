//! Node A loses a profile's lease to node B; A's writes through its fenced
//! view of the shared backend are refused while B's succeed.

use super::*;
use crate::storage::fence::{fence_error, FenceError, LeaseFence, ScopeMatch};
use crate::storage::lease::{DocumentLeases, LeaseGrant, LeaseStore, LocalLeases};
use crate::storage::MemoryStorage;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const TTL: Duration = Duration::from_secs(30);
const MARGIN_MS: u64 = 5_000;

/// One node: its clock, its lease store and fence registry, and its fenced
/// view of the shared backend.
struct Node {
    name: &'static str,
    clock: Arc<AtomicU64>,
    leases: Arc<dyn LeaseStore>,
    fences: Arc<FenceRegistry>,
    storage: FencedBackend,
}

impl Node {
    fn new(name: &'static str, shared: &Arc<dyn StorageBackend>) -> Self {
        let clock = Arc::new(AtomicU64::new(0));
        let reading = Arc::clone(&clock);
        let fences = Arc::new(FenceRegistry::new(move || reading.load(Ordering::SeqCst)));
        Self {
            name,
            leases: Arc::new(DocumentLeases::cluster(shared.as_ref(), name, None, TTL).unwrap()),
            storage: FencedBackend::new(Arc::clone(shared), Arc::clone(&fences)),
            fences,
            clock,
        }
    }

    fn set_clock(&self, ms: u64) {
        self.clock.store(ms, Ordering::SeqCst);
    }

    async fn take(&self, key: &str) -> LeaseGrant {
        let grant = self
            .leases
            .acquire(key, self.clock.load(Ordering::SeqCst))
            .await
            .unwrap();
        self.fences.register(Arc::new(LeaseFence::new(
            Arc::clone(&self.leases),
            key,
            self.name,
            grant.epoch,
            grant.expires_at_ms,
            MARGIN_MS,
            vec![
                ScopeMatch::Exact(format!("profile:{key}")),
                ScopeMatch::Prefix(format!("{key}~")),
            ],
        )));
        grant
    }

    fn scoped(&self, scope: &str) -> ScopedStorage {
        self.storage.for_scope(&Scope::new(scope).unwrap()).unwrap()
    }

    async fn put(&self, scope: &str, id: &str) -> Result<Version> {
        self.scoped(scope)
            .documents()
            .put("notes", id, json!({"by": self.name}), Precondition::None)
            .await
    }
}

fn shared() -> Arc<dyn StorageBackend> {
    Arc::new(MemoryStorage::new())
}

#[tokio::test]
async fn a_node_that_lost_the_lease_cannot_write_and_the_new_holder_can() {
    let backend = shared();
    let (a, b) = (Node::new("node-a", &backend), Node::new("node-b", &backend));

    a.take("alice").await;
    a.put("profile:alice", "n1").await.unwrap();
    a.put("alice~default", "n1").await.unwrap();

    // Node B's clock passes A's expiry and takes the profile over. Node A's
    // clock is still early (it was paused, or runs behind): its local check
    // passes, so the stored record is what stops it.
    b.set_clock(TTL.as_millis() as u64 + 1);
    let stolen = b.take("alice").await;
    assert!(stolen.previous_unclean);
    a.set_clock(1_000);

    let refused = a.put("profile:alice", "n2").await.unwrap_err();
    assert!(matches!(
        fence_error(&refused),
        Some(FenceError::Superseded {
            held: 1,
            stored: Some(2)
        })
    ));
    // Every write port, and every session scope of the profile, is fenced.
    let session = a.scoped("alice~default");
    assert!(session.streams().append("log", json!(1)).await.is_err());
    assert!(session.blobs().put("k", b"x".to_vec(), None).await.is_err());
    assert!(session
        .documents()
        .delete("notes", "n1", Precondition::None)
        .await
        .is_err());
    // Reads are not.
    assert!(session
        .documents()
        .get("notes", "n1")
        .await
        .unwrap()
        .is_some());
    // Other profiles' scopes are untouched.
    a.put("profile:bob", "n1").await.unwrap();

    // The new holder writes, and its write is the one that stands.
    b.put("profile:alice", "n2").await.unwrap();
    let stored = backend
        .for_scope(&Scope::new("profile:alice").unwrap())
        .unwrap()
        .documents()
        .get("notes", "n2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.doc["by"], "node-b");
}

#[tokio::test]
async fn a_paused_node_refuses_its_own_writes_once_its_grant_runs_out() {
    let backend = shared();
    let a = Node::new("node-a", &backend);
    a.take("alice").await;

    // Nobody took over yet, but node A's renewals stopped landing: inside
    // the skew margin it already refuses, with no I/O.
    a.set_clock(TTL.as_millis() as u64 - MARGIN_MS);
    let refused = a.put("profile:alice", "n").await.unwrap_err();
    assert_eq!(
        fence_error(&refused),
        Some(&FenceError::Expired { epoch: 1 })
    );
}

#[tokio::test]
async fn a_renewed_grant_keeps_admitting_writes() {
    let backend = shared();
    let a = Node::new("node-a", &backend);
    let grant = a.take("alice").await;
    a.set_clock(20_000);
    let renewed = a.leases.renew(&grant, 20_000).await.unwrap();
    a.fences
        .lookup("profile:alice")
        .unwrap()
        .renewed(renewed.expires_at_ms);
    a.set_clock(40_000);
    a.put("profile:alice", "n").await.unwrap();
}

/// File leases (`LocalLeases`, hosts without a backend) cannot be lost while
/// the process lives, so a fence over one admits writes until the holder
/// releases it.
#[tokio::test]
async fn a_fence_over_file_leases_admits_until_release() {
    let tmp = tempfile::tempdir().unwrap();
    let leases: Arc<dyn LeaseStore> = Arc::new(LocalLeases::new(tmp.path(), "node-a"));
    let grant = leases.acquire("alice", 0).await.unwrap();
    assert_eq!(grant.expires_at_ms, u64::MAX, "file leases never expire");
    let fence = LeaseFence::new(
        Arc::clone(&leases),
        "alice",
        "node-a",
        grant.epoch,
        grant.expires_at_ms,
        MARGIN_MS,
        vec![ScopeMatch::Exact("profile:alice".into())],
    );
    assert_eq!(fence.check(u64::MAX / 2).await, Ok(()));
    leases.release(grant).await.unwrap();
    assert!(matches!(
        fence.check(1).await,
        Err(FenceError::Superseded { .. })
    ));
}

/// Node A's lease store, with the pause the host check cannot see: its next
/// record read returns what it read and *then* lets node B take the key
/// over, as if A were paused between its check and its write.
struct PausedAfterCheck {
    inner: Arc<dyn LeaseStore>,
    thief: Arc<dyn LeaseStore>,
    armed: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl LeaseStore for PausedAfterCheck {
    async fn acquire(
        &self,
        key: &str,
        now_ms: u64,
    ) -> std::result::Result<LeaseGrant, crate::storage::lease::LeaseError> {
        self.inner.acquire(key, now_ms).await
    }

    async fn renew(
        &self,
        grant: &LeaseGrant,
        now_ms: u64,
    ) -> std::result::Result<LeaseGrant, crate::storage::lease::LeaseError> {
        self.inner.renew(grant, now_ms).await
    }

    async fn release(
        &self,
        grant: LeaseGrant,
    ) -> std::result::Result<(), crate::storage::lease::LeaseError> {
        self.inner.release(grant).await
    }

    async fn holder(
        &self,
        key: &str,
    ) -> std::result::Result<
        Option<crate::storage::lease::LeaseRecord>,
        crate::storage::lease::LeaseError,
    > {
        let record = self.inner.holder(key).await?;
        if self.armed.swap(false, Ordering::SeqCst) {
            let stolen = self.thief.acquire(key, TTL.as_millis() as u64 + 1).await?;
            assert_eq!(stolen.epoch, 2);
        }
        Ok(record)
    }

    fn record_scope(&self) -> Option<Scope> {
        self.inner.record_scope()
    }
}

/// Node A holds `alice` on `shared`; returns A's fenced view, its fence,
/// and the switch that makes its next check go stale.
async fn paused_holder(
    shared: &Arc<dyn StorageBackend>,
) -> (FencedBackend, Arc<LeaseFence>, Arc<PausedAfterCheck>) {
    let leases = Arc::new(PausedAfterCheck {
        inner: Arc::new(DocumentLeases::cluster(shared.as_ref(), "node-a", None, TTL).unwrap()),
        thief: Arc::new(DocumentLeases::cluster(shared.as_ref(), "node-b", None, TTL).unwrap()),
        armed: std::sync::atomic::AtomicBool::new(false),
    });
    let grant = leases.acquire("alice", 0).await.unwrap();
    let fences = Arc::new(FenceRegistry::new(|| 1_000));
    let fence = Arc::new(LeaseFence::new(
        Arc::clone(&leases) as Arc<dyn LeaseStore>,
        "alice",
        "node-a",
        grant.epoch,
        grant.expires_at_ms,
        MARGIN_MS,
        vec![ScopeMatch::Exact("profile:alice".into())],
    ));
    fences.register(Arc::clone(&fence));
    (
        FencedBackend::new(Arc::clone(shared), fences),
        fence,
        leases,
    )
}

/// The race the host check alone leaves open: A's check passes, A pauses,
/// B takes over, A's write lands. The driver re-reads the lease record in
/// the write's own atomic step and refuses it.
async fn driver_refuses_the_write_the_host_check_admitted(shared: Arc<dyn StorageBackend>) {
    assert!(shared.capabilities().contains(Capability::Fencing));
    let (a, fence, leases) = paused_holder(&shared).await;
    let scoped = a.for_scope(&Scope::new("profile:alice").unwrap()).unwrap();
    scoped
        .documents()
        .put("notes", "n1", json!({"v": 1}), Precondition::None)
        .await
        .unwrap();

    leases.armed.store(true, Ordering::SeqCst);
    let refused = scoped
        .documents()
        .put("notes", "n1", json!({"v": 2}), Precondition::None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            fence_error(&refused),
            Some(FenceError::Superseded {
                held: 1,
                stored: Some(2)
            })
        ),
        "{refused:?}"
    );
    assert!(fence.is_fenced(), "a driver refusal latches the fence");
    let stored = shared
        .for_scope(&Scope::new("profile:alice").unwrap())
        .unwrap()
        .documents()
        .get("notes", "n1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.doc["v"], 1, "the late write did not land");
    // Latched: every later write is refused before it reaches the driver.
    assert!(scoped.streams().append("log", json!(1)).await.is_err());
}

#[tokio::test]
async fn memory_driver_refuses_a_write_after_a_takeover_the_check_missed() {
    driver_refuses_the_write_the_host_check_admitted(shared()).await;
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn sqlite_driver_refuses_a_write_after_a_takeover_the_check_missed() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("fenced.db").display());
    let backend =
        tinystoragedrivers::open(&tinystoragedrivers::StorageConfig::parse(&url).unwrap())
            .await
            .unwrap();
    driver_refuses_the_write_the_host_check_admitted(backend).await;
}

/// Streams and blobs go through the driver fence too.
#[tokio::test]
async fn stream_and_blob_writes_are_driver_fenced() {
    let backend = shared();
    let (a, _fence, leases) = paused_holder(&backend).await;
    let scoped = a.for_scope(&Scope::new("profile:alice").unwrap()).unwrap();
    leases.armed.store(true, Ordering::SeqCst);
    assert!(scoped.streams().append("log", json!(1)).await.is_err());
    let raw = backend
        .for_scope(&Scope::new("profile:alice").unwrap())
        .unwrap();
    assert_eq!(raw.streams().len("log").await.unwrap(), 0);

    let (b, _fence, leases) = paused_holder(&shared()).await;
    let scoped = b.for_scope(&Scope::new("profile:alice").unwrap()).unwrap();
    leases.armed.store(true, Ordering::SeqCst);
    assert!(scoped.blobs().put("k", b"x".to_vec(), None).await.is_err());
}

/// A driver without fencing gets the host check only, and writes through.
#[tokio::test]
async fn a_driver_without_fencing_falls_back_to_the_host_check() {
    let backend: Arc<dyn StorageBackend> = Arc::new(NoFencing(MemoryStorage::new()));
    let (a, _fence, _leases) = paused_holder(&backend).await;
    let scoped = a.for_scope(&Scope::new("profile:alice").unwrap()).unwrap();
    scoped
        .documents()
        .put("notes", "n", json!({}), Precondition::None)
        .await
        .unwrap();
}

/// The memory driver, reporting no fencing (MongoDB without transactions).
#[derive(Debug)]
struct NoFencing(MemoryStorage);

impl StorageBackend for NoFencing {
    fn driver(&self) -> &'static str {
        "no-fencing"
    }
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities().without(Capability::Fencing)
    }
    fn for_scope(&self, scope: &Scope) -> Result<ScopedStorage> {
        self.0.for_scope(scope)
    }
}

/// The run ledger claims team tasks with `atomic_batch_checked`. The fenced
/// view must pass it through to a driver that supports it (the default trait
/// method answers `Unsupported`), and still refuse it for a stale lease.
#[tokio::test]
async fn a_checked_batch_reaches_the_driver_through_the_fenced_view() {
    use tinystoragedrivers::{ExpectedDocument, ReadCheck};

    let shared: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let a = Node::new("a", &shared);
    let docs = Arc::clone(a.scoped("plain").documents());
    docs.ensure_collection(&CollectionSpec::new("things"))
        .await
        .unwrap();

    let applied = docs
        .atomic_batch_checked(
            vec![ReadCheck {
                collection: "things".into(),
                id: "dependency".into(),
                expected: ExpectedDocument::Absent,
            }],
            vec![WriteOp::Put {
                collection: "things".into(),
                id: "claimed".into(),
                doc: json!({ "by": "a" }),
                precondition: Precondition::Absent,
            }],
        )
        .await
        .expect("the checked batch is not answered Unsupported");
    assert_eq!(applied.len(), 1);
    assert!(docs.get("things", "claimed").await.unwrap().is_some());

    // The dependency is validated: a check that no longer holds writes nothing.
    docs.put("things", "dependency", json!({}), Precondition::Absent)
        .await
        .unwrap();
    let conflict = docs
        .atomic_batch_checked(
            vec![ReadCheck {
                collection: "things".into(),
                id: "dependency".into(),
                expected: ExpectedDocument::Absent,
            }],
            vec![WriteOp::Put {
                collection: "things".into(),
                id: "second".into(),
                doc: json!({}),
                precondition: Precondition::Absent,
            }],
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.kind(), ErrorKind::Conflict);
    assert!(docs.get("things", "second").await.unwrap().is_none());
}

/// The fenced view must not hide what the driver can do: an expiring blob
/// written through it is stored, reads back, and the view advertises the
/// capability (before, `FencedBlobs` reported none and `put_expiring` fell to
/// the trait default, `Unsupported`, on every driver).
#[tokio::test]
async fn fenced_blobs_forward_expiry_and_capabilities() {
    let backend = shared();
    let node = Node::new("node-a", &backend);
    node.take("alice").await;
    let scoped = node.scoped("profile:alice");

    let blobs = scoped.blobs();
    assert!(blobs.capabilities().contains(Capability::BlobTtl));
    assert!(!blobs.capabilities().contains(Capability::PresignedUrls));
    assert_eq!(
        blobs.max_key_len(),
        backend
            .for_scope(&Scope::new("profile:alice").unwrap())
            .unwrap()
            .blobs()
            .max_key_len()
    );

    blobs
        .put_expiring("live", b"x".to_vec(), None, Duration::from_secs(3600))
        .await
        .unwrap();
    assert_eq!(blobs.get("live").await.unwrap().unwrap().bytes, b"x");
    assert!(blobs.get_stream("live").await.unwrap().is_some());

    // A zero lifetime is already expired: written, never visible.
    blobs
        .put_expiring("gone", b"x".to_vec(), None, Duration::ZERO)
        .await
        .unwrap();
    assert!(blobs.get("gone").await.unwrap().is_none());
    assert!(blobs.get_stream("gone").await.unwrap().is_none());
}

/// An expiring write is a write: a node that lost the lease cannot make one.
#[tokio::test]
async fn expiring_blob_writes_are_fenced() {
    let backend = shared();
    let (a, b) = (Node::new("node-a", &backend), Node::new("node-b", &backend));
    a.take("alice").await;
    b.set_clock(TTL.as_millis() as u64 + 1);
    b.take("alice").await;
    a.set_clock(1_000);

    let session = a.scoped("alice~default");
    assert!(session
        .blobs()
        .put_expiring("k", b"x".to_vec(), None, Duration::from_secs(60))
        .await
        .is_err());
    assert!(backend
        .for_scope(&Scope::new("alice~default").unwrap())
        .unwrap()
        .blobs()
        .get("k")
        .await
        .unwrap()
        .is_none());
}

/// The wrapper must pass a conditional append through to a driver that has it:
/// the trait default answers `Unsupported(ConditionalStreamAppend)`, which
/// refused every conversation append on the default (SQLite) backend.
async fn conditional_append_passes_through(inner: Arc<dyn StorageBackend>) {
    let wrapped = wrap(inner);
    let scoped = wrapped.for_scope(&Scope::local()).unwrap();
    let streams = scoped.streams();
    assert_eq!(
        streams
            .append_batch_if_offset("log", 0, vec![json!(1)])
            .await
            .unwrap(),
        0
    );
    let stale = streams
        .append_batch_if_offset("log", 0, vec![json!(2)])
        .await
        .unwrap_err();
    assert_eq!(stale.kind(), tinystoragedrivers::ErrorKind::Conflict);
    assert_eq!(streams.len("log").await.unwrap(), 1);
}

#[tokio::test]
async fn fenced_memory_backend_passes_conditional_appends_through() {
    conditional_append_passes_through(shared()).await;
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn fenced_sqlite_backend_passes_conditional_appends_through() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("cond.db").display());
    let backend =
        tinystoragedrivers::open(&tinystoragedrivers::StorageConfig::parse(&url).unwrap())
            .await
            .unwrap();
    conditional_append_passes_through(backend).await;
}
