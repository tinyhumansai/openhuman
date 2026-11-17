use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::FutureExt;
use tokio::sync::Notify;

use crate::core::runtime::{DomainSet, SaasConfig};
use crate::profiles::credentials::UserCredentialKind;
use crate::profiles::OpenError;
use crate::storage::lease::{LeaseRecord, LeaseStore};
use crate::storage::StorageBackend;

fn saas(root: &std::path::Path, node: &str) -> SaasConfig {
    let mut saas = SaasConfig::new(root);
    saas.node_id = Some(node.to_string());
    saas.idle_evict_secs = 3600;
    saas
}

/// A host for `node` on `root`: file registry and file-lock leases.
fn node(root: &std::path::Path, node: &str) -> ProfileHost {
    ProfileHost::new(
        saas(root, node),
        CoreContext::for_test(DomainSet::full(), None),
    )
}

/// A host for `node` on `root` over a shared storage backend.
fn clustered(root: &std::path::Path, node: &str, backend: &Arc<dyn StorageBackend>) -> ProfileHost {
    ProfileHost::with_backend(
        saas(root, node),
        CoreContext::for_test(DomainSet::full(), None),
        Some(Arc::clone(backend)),
    )
    .unwrap()
}

fn profile(name: &str) -> ProfileId {
    ProfileId::parse(name).unwrap()
}

/// A profile id no other test (sharing the process keyring) uses.
fn unique(prefix: &str) -> ProfileId {
    profile(&format!("{prefix}-{}", uuid::Uuid::new_v4().simple()))
}

/// Wraps a lease store; once armed, the next `acquire` signals `entered`
/// and waits for `go` before taking the lease. Lets a test stop an open
/// between its registry check and its lease.
struct PausingLeases {
    inner: Arc<dyn LeaseStore>,
    armed: AtomicBool,
    entered: Notify,
    go: Notify,
}

impl PausingLeases {
    fn wrap(inner: Arc<dyn LeaseStore>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            armed: AtomicBool::new(false),
            entered: Notify::new(),
            go: Notify::new(),
        })
    }
}

#[async_trait]
impl LeaseStore for PausingLeases {
    async fn acquire(&self, key: &str, now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.go.notified().await;
        }
        self.inner.acquire(key, now_ms).await
    }

    async fn renew(&self, grant: &LeaseGrant, now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        self.inner.renew(grant, now_ms).await
    }

    async fn release(&self, grant: LeaseGrant) -> Result<(), LeaseError> {
        self.inner.release(grant).await
    }

    async fn holder(&self, key: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        self.inner.holder(key).await
    }

    async fn holder_readonly(&self, key: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        self.inner.holder_readonly(key).await
    }
}

#[tokio::test]
async fn profile_locks_are_per_profile_and_forget_idle_ids() {
    let locks = ProfileLocks::default();
    let (a, b) = (profile("a"), profile("b"));
    let held_a = locks.lock(&a).await;
    let held_b = locks
        .lock(&b)
        .now_or_never()
        .expect("another profile is not held up");

    let waiting = locks.lock(&a);
    tokio::pin!(waiting);
    assert!(
        futures::poll!(&mut waiting).is_pending(),
        "the same profile waits"
    );
    drop(held_a);
    drop(waiting.await);
    drop(held_b);

    for i in 0..100 {
        drop(locks.lock(&profile(&format!("p{i}"))).await);
    }
    assert!(
        locks.tracked() <= 1,
        "ids nobody holds are forgotten: {}",
        locks.tracked()
    );
}

#[tokio::test]
async fn a_credential_install_waits_for_an_archive_in_flight_and_then_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let host = node(tmp.path(), "node-a");
    let id = unique("late-credential");
    host.provision(&id).await.unwrap();

    // A deprovision of this profile is in flight: it holds the profile's
    // lifecycle lock across clearing the credential and archiving.
    let archiving = host.lifecycle.lock(&id).await;
    let install = super::super::ops::set_credential_on(
        &host,
        id.as_str(),
        UserCredentialKind::Session,
        "late-session",
        None,
    );
    tokio::pin!(install);
    assert!(
        futures::poll!(&mut install).is_pending(),
        "a credential install must wait for the archive"
    );
    assert!(host.deprovision_locked(&id).await.unwrap());
    drop(archiving);

    let error = install.await.unwrap_err();
    assert!(error.contains("not provisioned"), "{error}");
    // Nothing was written into the archived profile's place or keyring.
    let layout = ProfileLayout::new(tmp.path(), &id);
    assert!(!layout.dir.exists(), "the profile directory stays archived");
    host.provision(&id).await.unwrap();
    assert!(
        !host.summary(&id).await.unwrap().unwrap().has_credential,
        "a re-provisioned profile starts without the late credential"
    );
}

#[tokio::test]
async fn an_open_racing_an_archive_on_another_node_does_not_resurrect_the_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = node(tmp.path(), "node-a");
    let pausing = PausingLeases::wrap(Arc::clone(&a.leases));
    a.leases = Arc::clone(&pausing) as Arc<dyn LeaseStore>;
    let a = Arc::new(a);
    let b = node(tmp.path(), "node-b");
    let id = profile("racer");
    b.provision(&id).await.unwrap();

    // Node a reads the record, then stops before taking the lease...
    pausing.armed.store(true, Ordering::SeqCst);
    let opening = tokio::spawn({
        let (a, id) = (Arc::clone(&a), id.clone());
        async move { a.open(&id).await.map(|_| ()) }
    });
    pausing.entered.notified().await;
    // ...while node b archives the profile.
    assert!(b.deprovision(&id).await.unwrap());
    pausing.go.notify_one();

    assert_eq!(
        opening.await.unwrap(),
        Err(OpenError::NotProvisioned(id.clone()))
    );
    assert!(!a.is_open(&id));
    let layout = ProfileLayout::new(tmp.path(), &id);
    assert!(
        !layout.workspace_dir.exists() && !layout.sandbox_dir.exists(),
        "the archived profile's directories are not recreated"
    );
}

#[tokio::test]
async fn a_failed_archive_gives_the_lease_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (node(tmp.path(), "node-a"), node(tmp.path(), "node-b"));
    let id = profile("stuck");
    a.provision(&id).await.unwrap();
    // The archive directory cannot be created.
    std::fs::write(layout::archive_dir(tmp.path()), b"not a directory").unwrap();

    a.deprovision(&id).await.unwrap_err();
    assert!(a.summary(&id).await.unwrap().is_some(), "still provisioned");
    let opened = b.open(&id).await;
    assert!(opened.is_ok(), "the lease was given back: {opened:?}");
}

#[tokio::test]
async fn provisioning_waits_out_another_nodes_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let backend_fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend: Arc<dyn StorageBackend> = Arc::clone(&backend_fixture.backend);
    let (a, b) = (
        clustered(tmp.path(), "node-a", &backend),
        clustered(tmp.path(), "node-b", &backend),
    );
    let id = profile("contended");
    // Node a is mid-change on the id (a deprovision between removing the
    // record and releasing the lease).
    let grant = a
        .leases()
        .acquire(id.as_str(), profile_lease::now_ms())
        .await
        .unwrap();

    let error = b.provision(&id).await.unwrap_err();
    assert!(error.contains("node-a"), "{error}");
    assert!(
        !ProfileLayout::new(tmp.path(), &id).dir.exists(),
        "nothing is laid out under another node's lease"
    );

    a.leases().release(grant).await.unwrap();
    assert!(b.provision(&id).await.unwrap());
    assert!(!b.provision(&id).await.unwrap());
}
