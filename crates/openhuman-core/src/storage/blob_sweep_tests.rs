use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use tinystoragedrivers::MemoryStorage;

/// A memory backend on a clock the test advances by hand.
fn backend_with_clock() -> (Arc<AtomicU64>, Arc<dyn StorageBackend>) {
    let now = Arc::new(AtomicU64::new(1_000));
    let clock = Arc::clone(&now);
    let backend: Arc<dyn StorageBackend> =
        Arc::new(MemoryStorage::with_clock(Arc::new(move || {
            clock.load(Ordering::SeqCst)
        })));
    (now, backend)
}

async fn put_expiring(backend: &Arc<dyn StorageBackend>, scope: &Scope, key: &str, ms: u64) {
    backend
        .for_scope(scope)
        .unwrap()
        .blobs()
        .put_expiring(key, b"x".to_vec(), None, Duration::from_millis(ms))
        .await
        .unwrap();
}

#[tokio::test]
async fn sweep_scope_reclaims_only_lapsed_blobs_of_that_scope() {
    let (now, backend) = backend_with_clock();
    let local = Scope::local();
    let other = crate::storage::scope_for_agent("blob-sweep-other");
    put_expiring(&backend, &local, "short", 10).await;
    put_expiring(&backend, &local, "long", 10_000).await;
    put_expiring(&backend, &other, "short", 10).await;

    now.store(2_000, Ordering::SeqCst);
    assert_eq!(sweep_scope(backend.as_ref(), &local).await, 1);
    assert_eq!(sweep_scope(backend.as_ref(), &local).await, 0, "idempotent");
    assert_eq!(
        sweep_scope(backend.as_ref(), &other).await,
        1,
        "the other scope's lapsed blob was untouched until its own sweep"
    );
    let still = backend.for_scope(&local).unwrap().blobs().get("long").await;
    assert!(still.unwrap().is_some(), "an unexpired blob survives");
}

#[tokio::test]
async fn sweep_all_visits_the_local_scope() {
    let (now, backend) = backend_with_clock();
    put_expiring(&backend, &Scope::local(), "gone", 10).await;
    now.store(2_000, Ordering::SeqCst);
    assert_eq!(sweep_all(Arc::clone(&backend)).await, 1);
    assert_eq!(sweep_all(backend).await, 0);
}

#[tokio::test(start_paused = true)]
async fn spawned_sweeper_waits_for_its_first_delay_then_keeps_ticking() {
    let handle = spawn(
        &crate::core::runtime::DispatchCtx::ambient(),
        Duration::from_secs(10),
        Duration::from_secs(20),
    );
    tokio::time::sleep(Duration::from_secs(65)).await;
    assert!(!handle.is_finished(), "the sweeper keeps running");
    handle.abort();
}

#[tokio::test]
async fn sweeping_through_the_fenced_backend_reaches_the_driver() {
    let (now, inner) = backend_with_clock();
    let scope = Scope::local();
    put_expiring(&inner, &scope, "gone", 10).await;
    now.store(2_000, Ordering::SeqCst);
    let fenced: Arc<dyn StorageBackend> =
        Arc::new(super::super::fenced_backend::FencedBackend::new(
            inner,
            Arc::new(crate::storage::fence::FenceRegistry::new(|| 0)),
        ));
    assert_eq!(sweep_scope(fenced.as_ref(), &scope).await, 1);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn module_blob_scopes_are_swept_alongside_the_acting_scope() {
    let (now, backend) = backend_with_clock();
    let module_scopes = crate::modules::expiring_blob_module_scopes(
        &crate::core::runtime::DispatchCtx::host_default(),
    );
    assert!(!module_scopes.is_empty(), "TinyJuice owns expiring blobs");
    put_expiring(&backend, &module_scopes[0], "tinyjuice_ccr/t", 10).await;
    now.store(2_000, Ordering::SeqCst);
    assert_eq!(sweep_all(backend).await, 1);
}

#[cfg(not(feature = "modules"))]
#[test]
fn no_module_blob_scopes_without_the_modules_feature() {
    assert!(crate::modules::expiring_blob_module_scopes(
        &crate::core::runtime::DispatchCtx::host_default()
    )
    .is_empty());
}

#[tokio::test]
async fn sweep_profiles_reclaims_each_profiles_own_scope() {
    let (now, backend) = backend_with_clock();
    let acme = crate::storage::scope_for_profile("acme");
    let globex = crate::storage::scope_for_profile("globex");
    put_expiring(&backend, &acme, "gone", 10).await;
    put_expiring(&backend, &acme, "kept", 10_000).await;
    put_expiring(&backend, &globex, "gone", 10).await;
    now.store(2_000, Ordering::SeqCst);

    let ids = vec!["acme".to_string(), "globex".to_string()];
    assert_eq!(sweep_profiles(backend.as_ref(), &ids).await, 2);
    assert_eq!(
        sweep_profiles(backend.as_ref(), &ids).await,
        0,
        "idempotent"
    );
    let kept = backend.for_scope(&acme).unwrap().blobs().get("kept").await;
    assert!(kept.unwrap().is_some(), "an unexpired blob survives");
}

#[tokio::test]
async fn sweep_profiles_skips_profiles_not_listed() {
    let (now, backend) = backend_with_clock();
    let other = crate::storage::scope_for_profile("not-listed");
    put_expiring(&backend, &other, "gone", 10).await;
    now.store(2_000, Ordering::SeqCst);
    assert_eq!(sweep_profiles(backend.as_ref(), &[]).await, 0);
    assert_eq!(sweep_scope(backend.as_ref(), &other).await, 1);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn sweep_profiles_reaches_the_modules_scopes_for_that_profile() {
    let (now, backend) = backend_with_clock();
    let module_scopes = crate::modules::expiring_blob_module_scopes_for("acme");
    assert!(!module_scopes.is_empty(), "TinyJuice owns expiring blobs");
    put_expiring(&backend, &module_scopes[0], "tinyjuice_ccr/t", 10).await;
    now.store(2_000, Ordering::SeqCst);
    assert_eq!(
        sweep_profiles(backend.as_ref(), &["acme".to_string()]).await,
        1
    );
}

/// A backend whose sweep never completes.
#[derive(Debug)]
struct HungSweep(Arc<dyn StorageBackend>);

#[async_trait::async_trait]
impl StorageBackend for HungSweep {
    fn driver(&self) -> &'static str {
        self.0.driver()
    }
    fn capabilities(&self) -> tinystoragedrivers::Capabilities {
        self.0.capabilities()
    }
    fn for_scope(
        &self,
        scope: &Scope,
    ) -> tinystoragedrivers::Result<tinystoragedrivers::ScopedStorage> {
        self.0.for_scope(scope)
    }
    async fn sweep_expired_blobs(&self, _scope: &Scope) -> tinystoragedrivers::Result<u64> {
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn a_hung_scope_sweep_times_out_and_counts_as_zero() {
    let (_now, inner) = backend_with_clock();
    let hung = HungSweep(inner);
    let removed = sweep_scope_within(&hung, &Scope::local(), Duration::from_secs(30)).await;
    assert_eq!(removed, 0);
}

#[tokio::test]
async fn a_hung_profile_listing_times_out_instead_of_stalling() {
    let hung = bounded_listing(
        Duration::from_millis(20),
        std::future::pending::<Result<Vec<String>, String>>(),
    )
    .await;
    assert!(hung.is_none());
    let ok = bounded_listing(Duration::from_secs(5), async { 7u8 }).await;
    assert_eq!(ok, Some(7));
}

#[cfg(feature = "modules")]
#[test]
fn module_blob_scopes_follow_the_handed_context_and_never_cross_tenants() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    let profile = |name: &str| {
        DispatchCtx::new(
            CoreContext::for_test(DomainSet::full(), None).derive_with(
                ContextOverlay::new(
                    crate::config::Config::default(),
                    DomainSet::kernel(),
                    Default::default(),
                )
                .profile(name),
            ),
        )
    };
    let alice = crate::modules::expiring_blob_module_scopes(&profile("blob-alice"));
    let bob = crate::modules::expiring_blob_module_scopes(&profile("blob-bob"));
    assert!(!alice.is_empty() && !bob.is_empty());
    assert!(alice.iter().all(|s| s.as_str().contains("blob-alice")));
    assert!(bob.iter().all(|s| s.as_str().contains("blob-bob")));
}
