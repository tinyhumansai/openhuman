use super::*;
use std::{ops::Range, sync::Arc, time::Duration};
use tinystoragedrivers_api::{BlobStore, Storage, StorageBackend};
use tinystoragedrivers_types::{Blob, BlobMeta, Capabilities, Result};

/// The TSD testkit memory engine uses its real clock; tests wait on the
/// backend's observable expiry instead of mutating a process-global clock.
fn backend_with_clock() -> Arc<dyn StorageBackend> {
    crate::storage::test_support::memory_backend()
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

async fn wait_until_expired(backend: &Arc<dyn StorageBackend>, scope: &Scope, key: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if backend
                .for_scope(scope)
                .unwrap()
                .blobs()
                .get(key)
                .await
                .unwrap()
                .is_none()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the TSD memory blob's recorded TTL eventually lapses");
}

#[tokio::test]
async fn sweep_scope_reclaims_only_lapsed_blobs_of_that_scope() {
    let backend = backend_with_clock();
    let local = Scope::new("local").unwrap();
    let other = crate::storage::scope_for_agent("blob-sweep-other").to_storage_scope();
    put_expiring(&backend, &local, "short", 10).await;
    put_expiring(&backend, &local, "long", 10_000).await;
    put_expiring(&backend, &other, "short", 10).await;

    wait_until_expired(&backend, &local, "short").await;
    // The other scope's TTL starts slightly later. Wait for its own expiry
    // before asserting that the local sweep left it available to its sweep.
    wait_until_expired(&backend, &other, "short").await;
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
    let backend = backend_with_clock();
    put_expiring(&backend, &Scope::new("local").unwrap(), "gone", 10).await;
    wait_until_expired(&backend, &Scope::new("local").unwrap(), "gone").await;
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
    let inner = backend_with_clock();
    let scope = Scope::new("local").unwrap();
    put_expiring(&inner, &scope, "gone", 10).await;
    wait_until_expired(&inner, &scope, "gone").await;
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
    let backend = backend_with_clock();
    let module_scopes = crate::modules::expiring_blob_module_scopes(
        &crate::core::runtime::DispatchCtx::host_default(),
    );
    assert!(!module_scopes.is_empty(), "TinyJuice owns expiring blobs");
    put_expiring(&backend, &module_scopes[0], "tinyjuice_ccr/t", 10).await;
    wait_until_expired(&backend, &module_scopes[0], "tinyjuice_ccr/t").await;
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
    let backend = backend_with_clock();
    let acme = crate::storage::scope_for_profile("acme").to_storage_scope();
    let globex = crate::storage::scope_for_profile("globex").to_storage_scope();
    put_expiring(&backend, &acme, "gone", 10).await;
    put_expiring(&backend, &acme, "kept", 10_000).await;
    put_expiring(&backend, &globex, "gone", 10).await;
    wait_until_expired(&backend, &acme, "gone").await;
    wait_until_expired(&backend, &globex, "gone").await;

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
    let backend = backend_with_clock();
    let other = crate::storage::scope_for_profile("not-listed").to_storage_scope();
    put_expiring(&backend, &other, "gone", 10).await;
    wait_until_expired(&backend, &other, "gone").await;
    assert_eq!(sweep_profiles(backend.as_ref(), &[]).await, 0);
    assert_eq!(sweep_scope(backend.as_ref(), &other).await, 1);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn sweep_profiles_reaches_the_modules_scopes_for_that_profile() {
    let backend = backend_with_clock();
    let module_scopes = crate::modules::expiring_blob_module_scopes_for("acme");
    assert!(!module_scopes.is_empty(), "TinyJuice owns expiring blobs");
    put_expiring(&backend, &module_scopes[0], "tinyjuice_ccr/t", 10).await;
    wait_until_expired(&backend, &module_scopes[0], "tinyjuice_ccr/t").await;
    assert_eq!(
        sweep_profiles(backend.as_ref(), &["acme".to_string()]).await,
        1
    );
}

/// A blob handle whose sweep never completes, backed by the real testkit
/// engine for all other operations.
#[derive(Debug)]
struct HungBlobs(Arc<dyn BlobStore>);

#[async_trait::async_trait]
impl BlobStore for HungBlobs {
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    fn max_key_len(&self) -> usize {
        self.0.max_key_len()
    }
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: Option<&str>) -> Result<BlobMeta> {
        self.0.put(key, bytes, content_type).await
    }
    async fn get(&self, key: &str) -> Result<Option<Blob>> {
        self.0.get(key).await
    }
    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<Vec<u8>>> {
        self.0.get_range(key, range).await
    }
    async fn head(&self, key: &str) -> Result<Option<BlobMeta>> {
        self.0.head(key).await
    }
    async fn delete(&self, key: &str) -> Result<bool> {
        self.0.delete(key).await
    }
    async fn list(&self, prefix: &str) -> Result<Vec<BlobMeta>> {
        self.0.list(prefix).await
    }
    async fn sweep_expired(&self) -> Result<u64> {
        std::future::pending().await
    }
}

/// A backend that exposes real testkit handles except for a pending blob sweep.
#[derive(Debug)]
struct HungSweep(Arc<dyn StorageBackend>);

#[async_trait::async_trait]
impl StorageBackend for HungSweep {
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    fn for_scope(&self, scope: &Scope) -> Result<Storage> {
        let storage = self.0.for_scope(scope)?;
        Ok(Storage::new(
            Arc::clone(storage.documents()),
            Arc::clone(storage.streams()),
            Arc::clone(storage.kv()),
            Arc::new(HungBlobs(Arc::clone(storage.blobs()))),
            Arc::clone(storage.queues()),
            Arc::clone(storage.secrets()),
        ))
    }
}

#[tokio::test(start_paused = true)]
async fn a_hung_scope_sweep_times_out_and_counts_as_zero() {
    let inner = backend_with_clock();
    let hung = HungSweep(inner);
    let removed = sweep_scope_within(
        &hung,
        &Scope::new("local").unwrap(),
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(removed, 0);
}

#[tokio::test]
async fn a_hung_profile_listing_times_out_instead_of_stalling() {
    let hung = bounded_listing(
        Duration::from_millis(20),
        std::future::pending::<std::result::Result<Vec<String>, String>>(),
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

#[test]
fn orphan_sweep_age_is_strict_and_unknown_ages_are_retained() {
    assert!(!orphan_age_is_eligible(None, 100));
    assert!(!orphan_age_is_eligible(Some(100), 100));
    assert!(orphan_age_is_eligible(Some(99), 100));
}

#[test]
fn orphan_sweep_refuses_empty_and_engine_owned_prefixes() {
    for prefix in ["", "_tsd/", "_tsd/sealed", "_tsd/sealed/stream/"] {
        assert_eq!(
            validate_orphan_sweep_prefix(prefix, 1024)
                .unwrap_err()
                .kind(),
            tinystoragedrivers_types::ErrorKind::InvalidInput,
            "prefix {prefix:?}"
        );
    }
    assert!(validate_orphan_sweep_prefix("user/attachments/", 1024).is_ok());
    assert_eq!(
        validate_orphan_sweep_prefix("a longer prefix", 3)
            .unwrap_err()
            .kind(),
        tinystoragedrivers_types::ErrorKind::InvalidInput
    );
}

#[test]
fn orphan_sweep_rejects_malformed_or_nonadvancing_listing_pages() {
    use tinystoragedrivers_types::{BlobListEntry, BlobListPage, BlobMeta};

    let page = BlobListPage {
        items: vec![BlobListEntry {
            meta: BlobMeta {
                key: "user/b".into(),
                len: 1,
                content_type: None,
            },
            written_at_ms: Some(1),
        }],
        next_after: Some("user/a".into()),
    };
    assert_eq!(
        validate_orphan_sweep_page("user/", None, &page, 1024)
            .unwrap_err()
            .kind(),
        tinystoragedrivers_types::ErrorKind::Backend
    );
    assert_eq!(
        validate_orphan_sweep_page("user/", Some("user/b"), &page, 1024)
            .unwrap_err()
            .kind(),
        tinystoragedrivers_types::ErrorKind::Backend
    );
}

#[tokio::test]
async fn orphan_sweep_pages_and_preserves_referenced_blobs() {
    use tinystoragedrivers_testkit::memory_storage;
    use tinystoragedrivers_types::Scope;

    let storage = memory_storage(&Scope::new("local").unwrap());
    let blobs = storage.blobs();
    for index in 0..=MAX_BLOB_LIST_PAGE_ITEMS {
        blobs
            .put(&format!("cleanup/{index:04}"), vec![index as u8], None)
            .await
            .unwrap();
    }
    let kept = format!("cleanup/{:04}", MAX_BLOB_LIST_PAGE_ITEMS);
    let removed = sweep_unreferenced_blobs(blobs.as_ref(), "cleanup/", u64::MAX, |key| {
        let kept = kept.clone();
        async move { Ok(key == kept) }
    })
    .await
    .unwrap();

    assert_eq!(removed, MAX_BLOB_LIST_PAGE_ITEMS as u64);
    assert!(blobs.get(&kept).await.unwrap().is_some());
    assert!(blobs.get("cleanup/0000").await.unwrap().is_none());
}

#[tokio::test]
async fn orphan_sweep_checks_references_only_for_eligible_blobs() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tinystoragedrivers_testkit::memory_storage;
    use tinystoragedrivers_types::Scope;

    let storage = memory_storage(&Scope::new("local").unwrap());
    let blobs = storage.blobs();
    blobs.put("cleanup/a", vec![1], None).await.unwrap();
    blobs.put("cleanup/b", vec![2], None).await.unwrap();
    let checked = AtomicUsize::new(0);

    let removed = sweep_unreferenced_blobs(blobs.as_ref(), "cleanup/", 0, |_| {
        checked.fetch_add(1, Ordering::SeqCst);
        async { Ok(false) }
    })
    .await
    .unwrap();

    assert_eq!(removed, 0);
    assert_eq!(checked.load(Ordering::SeqCst), 0);
    assert!(blobs.get("cleanup/a").await.unwrap().is_some());
    assert!(blobs.get("cleanup/b").await.unwrap().is_some());
}

#[tokio::test]
async fn orphan_sweep_stops_on_reference_error_after_preserving_prior_progress() {
    use tinystoragedrivers_testkit::memory_storage;
    use tinystoragedrivers_types::Scope;

    let storage = memory_storage(&Scope::new("local").unwrap());
    let blobs = storage.blobs();
    for key in ["cleanup/a", "cleanup/b", "cleanup/c"] {
        blobs.put(key, vec![1], None).await.unwrap();
    }

    let error = sweep_unreferenced_blobs(blobs.as_ref(), "cleanup/", u64::MAX, |key| async move {
        if key == "cleanup/b" {
            Err(StorageError::backend("reference lookup failed"))
        } else {
            Ok(false)
        }
    })
    .await
    .unwrap_err();

    assert_eq!(error.kind(), tinystoragedrivers_types::ErrorKind::Backend);
    assert!(blobs.get("cleanup/a").await.unwrap().is_none());
    assert!(blobs.get("cleanup/b").await.unwrap().is_some());
    assert!(blobs.get("cleanup/c").await.unwrap().is_some());
}

#[tokio::test]
async fn orphan_sweep_stops_on_listing_error_before_touching_blobs() {
    use std::sync::Arc;
    use tinystoragedrivers_testkit::{memory_storage, Failing, FaultPlan};
    use tinystoragedrivers_types::{ErrorKind, Scope};

    let storage = memory_storage(&Scope::new("local").unwrap());
    let blobs = Arc::clone(storage.blobs());
    blobs.put("cleanup/a", vec![1], None).await.unwrap();
    let faults = FaultPlan::new();
    faults.fail_operation_nth("blobs.list_page", 1, ErrorKind::Unavailable);
    let failing = Failing::with_plan(blobs.clone(), faults);

    let error = sweep_unreferenced_blobs(&failing, "cleanup/", u64::MAX, |_| async { Ok(false) })
        .await
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Unavailable);
    assert!(blobs.get("cleanup/a").await.unwrap().is_some());
}

#[tokio::test]
async fn orphan_sweep_stops_on_delete_error_after_preserving_prior_progress() {
    use std::sync::Arc;
    use tinystoragedrivers_testkit::{memory_storage, Failing, FaultPlan};
    use tinystoragedrivers_types::{ErrorKind, Scope};

    let storage = memory_storage(&Scope::new("local").unwrap());
    let blobs = Arc::clone(storage.blobs());
    for key in ["cleanup/a", "cleanup/b", "cleanup/c"] {
        blobs.put(key, vec![1], None).await.unwrap();
    }
    let faults = FaultPlan::new();
    faults.fail_operation_nth("blobs.delete", 2, ErrorKind::Unavailable);
    let failing = Failing::with_plan(blobs.clone(), faults);

    let error = sweep_unreferenced_blobs(&failing, "cleanup/", u64::MAX, |_| async { Ok(false) })
        .await
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Unavailable);
    assert!(blobs.get("cleanup/a").await.unwrap().is_none());
    assert!(blobs.get("cleanup/b").await.unwrap().is_some());
    assert!(blobs.get("cleanup/c").await.unwrap().is_some());
}
