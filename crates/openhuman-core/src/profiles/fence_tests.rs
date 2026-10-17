use super::*;
use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
use crate::profiles::lease::{now_ms, renew_once};
use crate::profiles::ProfileHost;
use crate::storage::fence::{fence_error, FenceError, FenceRegistry};
use crate::storage::fenced_backend::FencedBackend;
use crate::storage::lease::DocumentLeases;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use serde_json::json;
use tinystoragedrivers::Precondition;

const TTL_SECS: u64 = 30;

fn profile(name: &str) -> ProfileId {
    ProfileId::parse(name).unwrap()
}

/// A host for `node` over `backend`, with its own fence registry, and that
/// node's fenced view of the backend.
fn node(
    root: &std::path::Path,
    node: &str,
    backend: &Arc<dyn StorageBackend>,
) -> (Arc<ProfileHost>, FencedBackend) {
    let mut saas = SaasConfig::new(root);
    saas.node_id = Some(node.to_string());
    saas.lease_ttl_secs = TTL_SECS;
    saas.idle_evict_secs = 3600;
    let fences = Arc::new(FenceRegistry::new(now_ms));
    let host = ProfileHost::with_backend(
        saas,
        CoreContext::for_test(DomainSet::full(), None),
        Some(Arc::clone(backend)),
    )
    .unwrap()
    .with_fences(Arc::clone(&fences));
    (
        Arc::new(host),
        FencedBackend::new(Arc::clone(backend), fences),
    )
}

async fn write(storage: &FencedBackend, scope: &str) -> Result<(), crate::storage::StorageError> {
    storage
        .for_scope(&Scope::new(scope).unwrap())?
        .documents()
        .put("notes", "n", json!({}), Precondition::None)
        .await
        .map(|_| ())
}

#[test]
fn the_margin_is_a_sixth_of_the_ttl() {
    assert_eq!(skew_margin(Duration::from_secs(30)), 5_000);
}

#[test]
fn a_profiles_fence_covers_its_scope_and_its_sessions() {
    let id = profile("alice");
    let scopes = scopes_of(&id);
    let covers = |s: &str| {
        scopes.iter().any(|m| match m {
            ScopeMatch::Exact(e) => e == s,
            ScopeMatch::Prefix(p) => s.starts_with(p.as_str()),
        })
    };
    assert!(covers(crate::storage::scope_for_profile("alice").as_str()));
    let tenant = crate::core::runtime::Tenant {
        profile: Some("alice".into()),
        agent: None,
    };
    let _ = tenant;
    assert!(covers(
        crate::storage::scope_for_profile("alice")
            .agent("default")
            .unwrap()
            .as_str()
    ));
    assert!(covers(
        crate::storage::scope_for_profile("alice")
            .module("tinymcp")
            .unwrap()
            .as_str()
    ));
    assert!(!covers(
        crate::storage::scope_for_profile("alice2").as_str()
    ));
}

#[tokio::test]
async fn a_takeover_fences_the_old_holders_writes_before_its_heartbeat_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let (a, a_storage) = node(tmp.path(), "node-a", &backend);
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();
    let state = a.open(&alice).await.unwrap();
    write(&a_storage, "u:alice").await.unwrap();
    write(&a_storage, "u:alice/agent:default").await.unwrap();

    // Another node takes the profile over (by its clock the lease ran out).
    let thief = DocumentLeases::cluster(
        backend.as_ref(),
        "node-b",
        None,
        Duration::from_secs(TTL_SECS),
    )
    .unwrap();
    thief
        .acquire(alice.as_str(), now_ms() + 2 * TTL_SECS * 1_000)
        .await
        .unwrap();

    // Node A's next write is refused at once, and the profile is latched.
    let refused = write(&a_storage, "u:alice/agent:default")
        .await
        .unwrap_err();
    assert!(matches!(
        fence_error(&refused),
        Some(FenceError::Superseded { .. })
    ));
    assert!(state.is_fenced());
    // ... so new work is refused for it (`ensure_hosted` reads this latch)
    // even before the heartbeat closes it.
    assert!(state.lease_fence().check_local(now_ms()).is_err());

    // The heartbeat closes it; the latched fence stays registered (refusing
    // writes) while the profile's state is still held.
    let report = renew_once(&a).await;
    assert_eq!(report.fenced, vec![alice.clone()]);
    assert!(write(&a_storage, "u:alice").await.is_err());
    drop(state);
    // Nothing holds the fenced context any more: the tombstone is pruned.
    write(&a_storage, "u:alice").await.unwrap();
}

#[tokio::test]
async fn a_clean_release_retires_the_fence_and_a_reopen_registers_a_new_one() {
    let tmp = tempfile::tempdir().unwrap();
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let (a, _) = node(tmp.path(), "node-a", &backend);
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();
    let first = a.open(&alice).await.unwrap().lease_fence().epoch();
    assert_eq!(a.fences().lookup("u:alice").unwrap().epoch(), first);

    assert!(a.release(&alice).await.unwrap());
    assert!(a.fences().lookup("u:alice").is_none());

    let second = a.open(&alice).await.unwrap();
    assert!(second.lease_fence().epoch() > first);
    assert!(Arc::ptr_eq(
        &a.fences().lookup("u:alice/agent:default").unwrap(),
        second.lease_fence()
    ));

    // A renewal reaches the fence.
    assert_eq!(renew_once(&a).await.renewed, 1);
    assert!(second.lease_fence().check_local(now_ms()).is_ok());
}

#[tokio::test]
async fn background_jobs_are_not_started_for_a_superseded_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let (a, _) = node(tmp.path(), "node-a", &backend);
    let alice = profile("alice");
    a.provision(&alice).await.unwrap();
    let state = a.open(&alice).await.unwrap();
    DocumentLeases::cluster(
        backend.as_ref(),
        "node-b",
        None,
        Duration::from_secs(TTL_SECS),
    )
    .unwrap()
    .acquire(alice.as_str(), now_ms() + 2 * TTL_SECS * 1_000)
    .await
    .unwrap();
    // The check the background loop runs before a profile's jobs.
    assert!(state.lease_fence().check(now_ms()).await.is_err());
    tokio::time::timeout(Duration::from_secs(1), state.lease_fence().fenced())
        .await
        .expect("running work observes the latch");
}
