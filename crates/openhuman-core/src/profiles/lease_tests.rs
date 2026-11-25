#![cfg(feature = "storage-sqlite")]

use super::*;
use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};

const TTL_SECS: u64 = 30;

fn saas(root: &std::path::Path, node: &str) -> SaasConfig {
    let mut saas = SaasConfig::new(root);
    saas.node_id = Some(node.to_string());
    saas.advertise_url = Some(format!("http://{node}.internal:7788"));
    saas.lease_ttl_secs = TTL_SECS;
    saas.idle_evict_secs = 3600;
    saas
}

/// A host on `root` for `node`, over `backend` when given.
fn node(
    root: &std::path::Path,
    node: &str,
    backend: Option<&Arc<dyn StorageBackend>>,
) -> Arc<ProfileHost> {
    Arc::new(
        ProfileHost::with_backend(
            saas(root, node),
            CoreContext::for_test(DomainSet::full(), None),
            backend.cloned(),
        )
        .unwrap(),
    )
}

fn shared() -> crate::storage::test_support::SqliteBackend {
    crate::storage::test_support::sqlite_backend().unwrap()
}

fn profile(name: &str) -> ProfileId {
    ProfileId::parse(name).unwrap()
}

#[test]
fn renewal_runs_three_times_per_ttl() {
    assert_eq!(
        renew_interval(Duration::from_secs(30)),
        Duration::from_secs(10)
    );
    assert_eq!(
        renew_interval(Duration::from_millis(30)),
        Duration::from_millis(100)
    );
}

#[tokio::test]
async fn a_profile_open_on_one_node_is_refused_on_another_naming_the_holder() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = shared();
    let (a, b) = (
        node(tmp.path(), "node-a", Some(&backend.backend)),
        node(tmp.path(), "node-b", Some(&backend.backend)),
    );
    let alice = profile("alice");
    assert!(a.provision(&alice).await.unwrap());
    assert!(
        !b.provision(&alice).await.unwrap(),
        "the registry is shared: node b sees alice provisioned"
    );
    assert_eq!(b.list().await.unwrap().len(), 1);

    let _held = a.open(&alice).await.unwrap();
    match b.open(&alice).await {
        Err(OpenError::HeldElsewhere(record)) => {
            assert_eq!(record.owner, "node-a");
            assert_eq!(
                record.endpoint.as_deref(),
                Some("http://node-a.internal:7788")
            );
            assert!(record.retry_after_ms(now_ms()) > 0);
        }
        other => panic!("expected HeldElsewhere, got {other:?}"),
    }
    assert!(!b.is_open(&alice));
    assert!(
        b.deprovision(&alice)
            .await
            .unwrap_err()
            .contains("hosted by node node-a"),
        "a profile hosted elsewhere is not archived from under it"
    );
}

#[tokio::test]
async fn release_hands_a_profile_to_another_node_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = shared();
    let (a, b) = (
        node(tmp.path(), "node-a", Some(&backend.backend)),
        node(tmp.path(), "node-b", Some(&backend.backend)),
    );
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();

    let held = a.open(&alice).await.unwrap();
    assert!(
        a.release(&alice).await.unwrap_err().contains("in use"),
        "a profile in use is not released from under its work"
    );
    drop(held);
    assert!(a.release(&alice).await.unwrap());
    assert!(!a.is_open(&alice));
    assert!(!a.release(&alice).await.unwrap(), "nothing left to release");

    let taken = b.open(&alice).await.unwrap();
    assert_eq!(taken.id, alice);
    assert!(matches!(
        a.open(&alice).await,
        Err(OpenError::HeldElsewhere(_))
    ));
}

#[tokio::test]
async fn eviction_and_shutdown_release_the_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = shared();
    let mut config = saas(tmp.path(), "node-a");
    config.idle_evict_secs = 0;
    let a = ProfileHost::with_backend(
        config,
        CoreContext::for_test(DomainSet::full(), None),
        Some(Arc::clone(&backend.backend)),
    )
    .unwrap();
    let b = node(tmp.path(), "node-b", Some(&backend.backend));
    let (alice, bob) = (profile("alice"), profile("bob"));
    a.provision(&alice).await.unwrap();
    a.provision(&bob).await.unwrap();

    drop(a.open(&alice).await.unwrap());
    a.evict_idle().await;
    assert!(!a.is_open(&alice));
    drop(b.open(&alice).await.unwrap());

    let busy = a.open(&bob).await.unwrap();
    a.release_idle_on_shutdown().await;
    assert!(
        a.is_open(&bob),
        "a profile in use keeps its lease at shutdown"
    );
    drop(busy);
    a.release_idle_on_shutdown().await;
    assert!(!a.is_open(&bob));
    drop(b.open(&bob).await.unwrap());
}

#[tokio::test]
async fn renewal_keeps_the_lease_and_a_lost_lease_fences_the_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = shared();
    let a = node(tmp.path(), "node-a", Some(&backend.backend));
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();
    let state = a.open(&alice).await.unwrap();

    let report = renew_once(&a).await;
    assert_eq!(report.renewed, 1);
    assert!(report.fenced.is_empty());

    // A turn is running on alice's context.
    let turn = tokio_util::sync::CancellationToken::new();
    CoreContext::scope(
        Arc::clone(state.context()),
        crate::web_chat::track_parallel_turn_for_test(
            &crate::core::runtime::DispatchCtx::new(Arc::clone(state.context())),
            "t1",
            turn.clone(),
        ),
    )
    .await;
    assert_eq!(
        CoreContext::scope(Arc::clone(state.context()), async {
            crate::core::runtime::current_tenant().unwrap().profile
        })
        .await
        .as_deref(),
        Some("alice")
    );

    // Another node takes the profile over once the lease has (by its clock)
    // run out.
    let thief = crate::storage::lease::DocumentLeases::cluster(
        backend.backend.as_ref(),
        "node-b",
        None,
        Duration::from_secs(TTL_SECS),
    )
    .unwrap();
    let stolen = thief
        .acquire(alice.as_str(), now_ms() + 2 * TTL_SECS * 1_000)
        .await
        .unwrap();
    assert!(stolen.previous_unclean);

    let mut events = crate::web_chat::subscribe_web_channel_events();
    let report = renew_once(&a).await;
    assert_eq!(report.fenced, vec![alice.clone()]);
    assert!(state.is_fenced(), "the profile is fenced");
    assert!(!a.is_open(&alice), "and closed");
    // The turn unwinds through the runner, which announces it afterwards.
    tokio::time::timeout(Duration::from_secs(10), turn.cancelled())
        .await
        .expect("its in-flight turns are stopped");
    // The cancelled event goes to the client that started the turn, so its
    // stream resolves rather than staying "running".
    let cancelled_for = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(event)
                    if event.event_name() == "chat_cancelled"
                        && event.context().thread_id == "t1" =>
                {
                    return event.context().client_id.clone();
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(error) => panic!("event stream closed: {error}"),
            }
        }
    })
    .await
    .ok();
    assert_eq!(cancelled_for.as_deref(), Some("test-client"));
}

#[tokio::test]
async fn a_fenced_profile_starts_no_new_work() {
    let tmp = tempfile::tempdir().unwrap();
    let a = node(tmp.path(), "node-a", None);
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();
    let state = a.open(&alice).await.unwrap();
    let grant = a.grants().pop().unwrap().1;

    // A stale grant (the slot moved on) fences nothing.
    let stale = crate::storage::lease::LeaseGrant {
        epoch: grant.epoch + 1,
        ..grant.clone()
    };
    a.fence(&alice, &stale, "test").await;
    assert!(a.is_open(&alice) && !state.is_fenced());

    a.fence(&alice, &grant, "test").await;
    assert!(state.is_fenced());
    assert!(!a.is_open(&alice));
}

#[tokio::test]
async fn file_leases_keep_two_hosts_on_one_root_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (
        node(tmp.path(), "node-a", None),
        node(tmp.path(), "node-b", None),
    );
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();
    let _held = a.open(&alice).await.unwrap();
    match b.open(&alice).await {
        Err(OpenError::HeldElsewhere(record)) => {
            assert_eq!(record.owner, "node-a");
            assert_eq!(
                record.endpoint.as_deref(),
                Some("http://node-a.internal:7788")
            );
        }
        other => panic!("expected HeldElsewhere, got {other:?}"),
    }
}

/// Wraps a lease store; once armed, the next `renew` completes in the inner
/// store (which then holds only the renewed grant), signals `entered` and
/// waits for `go` before returning, so the heartbeat has renewed but not yet
/// stored the new grant in the slot.
struct PausingRenewals {
    inner: Arc<dyn LeaseStore>,
    armed: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    go: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl LeaseStore for PausingRenewals {
    async fn acquire(&self, key: &str, now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        self.inner.acquire(key, now_ms).await
    }

    async fn renew(&self, grant: &LeaseGrant, now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        let renewed = self.inner.renew(grant, now_ms).await;
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.go.notified().await;
        }
        renewed
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

/// An operator release that lands while the heartbeat is renewing the same
/// profile must release the renewed grant, not fail with "lease lost" on
/// the one the renewal just superseded.
#[tokio::test]
async fn a_release_during_a_heartbeat_renewal_releases_the_renewed_grant() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = shared();
    let mut a = ProfileHost::with_backend(
        saas(tmp.path(), "node-a"),
        CoreContext::for_test(DomainSet::full(), None),
        Some(Arc::clone(&backend.backend)),
    )
    .unwrap();
    let pausing = Arc::new(PausingRenewals {
        inner: Arc::clone(&a.leases),
        armed: std::sync::atomic::AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        go: tokio::sync::Notify::new(),
    });
    a.leases = Arc::clone(&pausing) as Arc<dyn LeaseStore>;
    let a = Arc::new(a);
    let b = node(tmp.path(), "node-b", Some(&backend.backend));
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();
    drop(a.open(&alice).await.unwrap());

    // The heartbeat renews alice and stops before storing the new grant...
    pausing
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let heartbeat = tokio::spawn({
        let a = Arc::clone(&a);
        async move { renew_once(&a).await }
    });
    pausing.entered.notified().await;
    // ...while the operator releases alice.
    let release = tokio::spawn({
        let (a, alice) = (Arc::clone(&a), alice.clone());
        async move { a.release(&alice).await }
    });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    pausing.go.notify_one();

    let report = heartbeat.await.unwrap();
    assert!(report.fenced.is_empty(), "{report:?}");
    assert_eq!(release.await.unwrap(), Ok(true));
    assert!(!a.is_open(&alice));
    let record = a.leases().holder(alice.as_str()).await.unwrap().unwrap();
    assert!(
        record.released,
        "the record is released cleanly: {record:?}"
    );
    let taken = b.open(&alice).await.unwrap();
    assert_eq!(taken.id, alice);
}
