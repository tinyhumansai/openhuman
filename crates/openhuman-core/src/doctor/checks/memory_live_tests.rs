use super::{failure_finding, MemoryRoundtripCheck, Operation};
use crate::config::Config;
use crate::core::runtime::{CoreContext, DomainSet};
use crate::core::slots::Extensions;
use crate::doctor::contract::{DoctorCheck, DoctorCtx, FindingSeverity};
use crate::memory::test_fixtures::stored;
use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tempfile::TempDir;
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    EngineDescriptor, EngineHealth, Error as MemoryApiError, FetchMode, FetchPage, FetchRequest,
    ForgetReport, ForgetTarget, ItemId, ListPage, ListRequest, MemoryEngine, MemoryMeta,
    MetaFilter, Namespace, Reach, RecallAnswer, RecallRequest, StoreItem, StoreReceipt,
};

#[derive(Clone)]
enum Failure {
    None,
    Fetch,
    FetchUntilCancelled,
    StoreUntilCancelled,
    StoreAfterAccept(MemoryApiError),
}

#[derive(Default)]
struct Calls {
    steps: Vec<&'static str>,
    fetch_modes: Vec<FetchMode>,
    stored: Vec<(ItemId, Namespace)>,
    scoped_forgets: Vec<(Vec<ItemId>, Reach)>,
    unscoped_forgets: usize,
}

struct ProbeEngine {
    inner: ReferenceEngine,
    failure: Failure,
    calls: Mutex<Calls>,
    store_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    fetch_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    cleanup_finished: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl ProbeEngine {
    fn new(failure: Failure) -> Self {
        Self {
            inner: ReferenceEngine::new(),
            failure,
            calls: Mutex::new(Calls::default()),
            store_started: Mutex::new(None),
            fetch_started: Mutex::new(None),
            cleanup_finished: Mutex::new(None),
        }
    }

    fn pending_fetch() -> (
        Self,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let (fetch_started_tx, fetch_started_rx) = tokio::sync::oneshot::channel();
        let (cleanup_finished_tx, cleanup_finished_rx) = tokio::sync::oneshot::channel();
        (
            Self {
                inner: ReferenceEngine::new(),
                failure: Failure::FetchUntilCancelled,
                calls: Mutex::new(Calls::default()),
                store_started: Mutex::new(None),
                fetch_started: Mutex::new(Some(fetch_started_tx)),
                cleanup_finished: Mutex::new(Some(cleanup_finished_tx)),
            },
            fetch_started_rx,
            cleanup_finished_rx,
        )
    }

    fn pending_store() -> (
        Self,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let (store_started_tx, store_started_rx) = tokio::sync::oneshot::channel();
        let (cleanup_finished_tx, cleanup_finished_rx) = tokio::sync::oneshot::channel();
        (
            Self {
                inner: ReferenceEngine::new(),
                failure: Failure::StoreUntilCancelled,
                calls: Mutex::new(Calls::default()),
                store_started: Mutex::new(Some(store_started_tx)),
                fetch_started: Mutex::new(None),
                cleanup_finished: Mutex::new(Some(cleanup_finished_tx)),
            },
            store_started_rx,
            cleanup_finished_rx,
        )
    }
}

#[async_trait]
impl MemoryEngine for ProbeEngine {
    fn descriptor(&self) -> &EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> EngineHealth {
        self.inner.health().await
    }

    async fn recall(&self, request: RecallRequest) -> tinymemory_api::Result<RecallAnswer> {
        self.inner.recall(request).await
    }

    async fn fetch(&self, request: FetchRequest) -> tinymemory_api::Result<FetchPage> {
        {
            let mut calls = self.calls.lock().unwrap();
            calls.steps.push("fetch");
            calls.fetch_modes.push(request.mode);
        }
        if matches!(&self.failure, Failure::Fetch) {
            return Err(MemoryApiError::Unavailable("mock fetch failure".into()));
        }
        if matches!(&self.failure, Failure::FetchUntilCancelled) {
            if let Some(started) = self.fetch_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            return std::future::pending().await;
        }
        self.inner.fetch(request).await
    }

    async fn store(&self, item: StoreItem) -> tinymemory_api::Result<StoreReceipt> {
        self.calls.lock().unwrap().steps.push("store");
        let id = ItemId::new(item.fingerprint());
        let namespace = item.meta().namespace.clone();
        let receipt = self.inner.store(item).await?;
        self.calls.lock().unwrap().stored.push((id, namespace));
        if matches!(&self.failure, Failure::StoreUntilCancelled) {
            if let Some(started) = self.store_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            return std::future::pending().await;
        }
        if let Failure::StoreAfterAccept(error) = &self.failure {
            return Err(error.clone());
        }
        Ok(receipt)
    }

    async fn forget(&self, target: ForgetTarget) -> tinymemory_api::Result<ForgetReport> {
        self.calls.lock().unwrap().unscoped_forgets += 1;
        self.inner.forget(target).await
    }

    async fn forget_within(
        &self,
        ids: Vec<ItemId>,
        reach: Reach,
    ) -> tinymemory_api::Result<ForgetReport> {
        {
            let mut calls = self.calls.lock().unwrap();
            calls.steps.push("forget_within");
            calls.scoped_forgets.push((ids.clone(), reach.clone()));
        }
        let result = self.inner.forget_within(ids, reach).await;
        if let Some(finished) = self.cleanup_finished.lock().unwrap().take() {
            let _ = finished.send(());
        }
        result
    }

    async fn list(&self, request: ListRequest) -> tinymemory_api::Result<ListPage> {
        self.inner.list(request).await
    }
}

fn config_in(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

async fn seed_real_memory(engine: &ProbeEngine) -> ItemId {
    let item = StoreItem::document("existing user memory", MemoryMeta::default());
    let id = ItemId::new(item.fingerprint());
    engine
        .inner
        .store(item)
        .await
        .expect("seed existing memory");
    id
}

async fn run_check(
    engine: &Arc<ProbeEngine>,
    tmp: &TempDir,
) -> Vec<crate::doctor::contract::Finding> {
    run_check_with_config(engine, config_in(tmp)).await
}

async fn run_check_with_config(
    engine: &Arc<ProbeEngine>,
    mut config: Config,
) -> Vec<crate::doctor::contract::Finding> {
    config.memory.engine = crate::memory::engine::TINYHUMANS_ENGINE.to_owned();
    let extensions = Extensions::new();
    let engine_for_factory: Arc<dyn MemoryEngine> = engine.clone();
    crate::memory::engine_registry::register_read_only_engine_factory_in(
        &extensions,
        crate::memory::engine::TINYHUMANS_ENGINE,
        Arc::new(move |_selected| {
            let engine = Arc::clone(&engine_for_factory);
            Box::pin(async move { Ok(engine) })
        }),
    );
    let context = CoreContext::for_test_with_extensions(DomainSet::full(), None, extensions);
    CoreContext::scope(context, async {
        MemoryRoundtripCheck
            .detect(&DoctorCtx::from_config(config))
            .await
    })
    .await
}

async fn assert_scratch_namespaces_are_empty(engine: &ProbeEngine, namespaces: &[Namespace]) {
    for namespace in namespaces {
        let items = stored(
            &engine.inner,
            MetaFilter {
                reach: Some(Reach::subtree(namespace.clone())),
                ..MetaFilter::default()
            },
        )
        .await;
        assert!(items.is_empty(), "scratch item remained in {namespace}");
    }
}

fn only_error(findings: &[crate::doctor::contract::Finding]) -> &crate::doctor::contract::Finding {
    findings
        .iter()
        .find(|finding| finding.severity == FindingSeverity::Error)
        .expect("the live memory failure is reported as an error")
}

#[tokio::test]
async fn fetch_failure_still_cleans_the_unique_service_scoped_item() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let config = config_in(&tmp);
    let engine = Arc::new(ProbeEngine::new(Failure::Fetch));
    let real_id = seed_real_memory(&engine).await;
    let findings = run_check_with_config(&engine, config).await;

    assert!(only_error(&findings).message.contains("fetch"));
    let namespaces = {
        let calls = engine.calls.lock().unwrap();
        assert_eq!(calls.steps, ["store", "fetch", "forget_within"]);
        assert_eq!(
            calls.unscoped_forgets, 0,
            "cleanup is confined to the probe scope"
        );
        assert_eq!(calls.stored.len(), 1);
        let (scratch_id, namespace) = &calls.stored[0];
        assert_ne!(scratch_id, &real_id);
        assert_ne!(scratch_id.as_str(), "doctor:probe");
        assert!(namespace
            .to_string()
            .starts_with("service:openhuman-doctor-"));
        assert!(!Reach::subtree(Namespace::ROOT).admits(namespace));
        assert_eq!(calls.scoped_forgets.len(), 1);
        assert_eq!(calls.scoped_forgets[0].0, vec![scratch_id.clone()]);
        assert_eq!(calls.scoped_forgets[0].1, Reach::exact(namespace.clone()));
        calls
            .stored
            .iter()
            .map(|(_, namespace)| namespace.clone())
            .collect::<Vec<_>>()
    };

    assert_scratch_namespaces_are_empty(&engine, &namespaces).await;
    let remaining = stored(&engine.inner, MetaFilter::default()).await;
    assert_eq!(
        remaining.len(),
        1,
        "only the pre-existing user memory remains"
    );
    assert_eq!(remaining[0].id, real_id);
}

#[tokio::test]
async fn ambiguous_store_failure_attempts_scoped_cleanup_and_reports_a_billing_hint() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let config = config_in(&tmp);
    let engine = Arc::new(ProbeEngine::new(Failure::StoreAfterAccept(
        MemoryApiError::Engine(
            "[USER_INSUFFICIENT_CREDITS] mock account has insufficient credits (HTTP 402)".into(),
        ),
    )));
    let real_id = seed_real_memory(&engine).await;
    let findings = run_check_with_config(&engine, config).await;

    let finding = only_error(&findings);
    assert!(
        finding.fix_hint.to_ascii_lowercase().contains("credit")
            || finding.fix_hint.to_ascii_lowercase().contains("billing"),
        "insufficient credits needs an actionable billing hint: {}",
        finding.fix_hint
    );
    let namespaces = {
        let calls = engine.calls.lock().unwrap();
        assert_eq!(calls.steps, ["store", "forget_within"]);
        assert_eq!(calls.unscoped_forgets, 0);
        assert_eq!(calls.scoped_forgets.len(), 1);
        assert_eq!(calls.scoped_forgets[0].0, vec![calls.stored[0].0.clone()]);
        calls
            .stored
            .iter()
            .map(|(_, namespace)| namespace.clone())
            .collect::<Vec<_>>()
    };

    assert_scratch_namespaces_are_empty(&engine, &namespaces).await;
    let remaining = stored(&engine.inner, MetaFilter::default()).await;
    assert_eq!(
        remaining.len(),
        1,
        "cleanup removes a possibly accepted write"
    );
    assert_eq!(remaining[0].id, real_id);
}

#[test]
fn plan_required_memory_failure_has_safe_plan_guidance_and_cleanup_warning() {
    let finding = failure_finding(
        MemoryApiError::Engine(
            "[MEMORY_PLAN_REQUIRED] memory API private-host.example: diagnostic details".into(),
        ),
        Operation::Store,
        false,
    );

    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding
        .message
        .contains("not included in the selected account's plan"));
    assert!(!finding.message.contains("MEMORY_PLAN_REQUIRED"));
    assert!(!finding.message.contains("private-host.example"));
    assert!(finding
        .fix_hint
        .contains("Upgrade to a plan that includes memory"));
    assert!(finding
        .fix_hint
        .contains("could not confirm scratch cleanup"));
}

#[tokio::test]
async fn unauthorized_and_unavailable_errors_have_safe_recovery_hints() {
    let cases = [
        (
            MemoryApiError::Unauthorized("secret token must not be echoed".into()),
            ["sign in", "auth", "credential"],
        ),
        (
            MemoryApiError::Unavailable("private endpoint detail must not be echoed".into()),
            ["retry", "available", "service"],
        ),
    ];

    for (error, hint_words) in cases {
        let tmp = TempDir::new().expect("temporary config and workspace");
        let engine = Arc::new(ProbeEngine::new(Failure::StoreAfterAccept(error)));
        let findings = run_check(&engine, &tmp).await;
        let finding = only_error(&findings);
        let hint = finding.fix_hint.to_ascii_lowercase();
        assert!(
            hint_words.iter().any(|word| hint.contains(word)),
            "error needs a safe, relevant recovery hint: {}",
            finding.fix_hint
        );
        assert!(!finding.message.contains("secret token"));
        assert!(!finding.message.contains("private endpoint"));
        assert!(!finding.fix_hint.contains("secret token"));
        assert!(!finding.fix_hint.contains("private endpoint"));
        assert_eq!(engine.calls.lock().unwrap().unscoped_forgets, 0);
    }
}

#[tokio::test]
async fn each_successful_probe_uses_a_fresh_service_namespace_and_preserves_user_memory() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let config = config_in(&tmp);
    let engine = Arc::new(ProbeEngine::new(Failure::None));
    let real_id = seed_real_memory(&engine).await;

    for _ in 0..2 {
        let findings = run_check_with_config(&engine, config.clone()).await;
        assert!(
            findings
                .iter()
                .all(|finding| finding.severity != FindingSeverity::Error),
            "a successful store/fetch/forget roundtrip passes"
        );
    }

    let namespaces = {
        let calls = engine.calls.lock().unwrap();
        assert_eq!(
            calls.steps,
            [
                "store",
                "fetch",
                "forget_within",
                "store",
                "fetch",
                "forget_within"
            ]
        );
        assert_eq!(calls.fetch_modes, [FetchMode::Hybrid, FetchMode::Hybrid]);
        assert_eq!(calls.stored.len(), 2);
        assert_ne!(
            calls.stored[0].0, calls.stored[1].0,
            "probe IDs are unique per run"
        );
        assert_ne!(
            calls.stored[0].1, calls.stored[1].1,
            "probe namespaces are unique per run"
        );
        assert!(calls.stored.iter().all(|(_, namespace)| {
            namespace
                .to_string()
                .starts_with("service:openhuman-doctor-")
                && !Reach::subtree(Namespace::ROOT).admits(namespace)
        }));
        assert_eq!(calls.unscoped_forgets, 0);
        assert_eq!(calls.scoped_forgets.len(), 2);
        calls
            .stored
            .iter()
            .map(|(_, namespace)| namespace.clone())
            .collect::<Vec<_>>()
    };

    assert_scratch_namespaces_are_empty(&engine, &namespaces).await;
    let remaining = stored(&engine.inner, MetaFilter::default()).await;
    assert_eq!(remaining.len(), 1, "the scratch probes are gone");
    assert_eq!(remaining[0].id, real_id, "unrelated memory is preserved");
}

#[test]
fn probe_fetch_mode_prefers_hybrid_then_falls_back_to_keyword() {
    let engine = ReferenceEngine::new();
    let mut descriptor = engine.descriptor().clone();
    descriptor.fetch_modes = vec![FetchMode::Keyword, FetchMode::Hybrid];
    assert_eq!(
        super::probe_fetch_mode(&descriptor),
        Some(FetchMode::Hybrid)
    );

    descriptor.fetch_modes = vec![FetchMode::Keyword];
    assert_eq!(
        super::probe_fetch_mode(&descriptor),
        Some(FetchMode::Keyword)
    );

    descriptor.fetch_modes = vec![FetchMode::Vector];
    assert_eq!(super::probe_fetch_mode(&descriptor), None);
}

#[tokio::test]
async fn cancelling_a_pending_fetch_keeps_scoped_cleanup_owned_by_the_guard() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let config = config_in(&tmp);
    let (probe, fetch_started, cleanup_finished) = ProbeEngine::pending_fetch();
    let engine = Arc::new(probe);
    let real_id = seed_real_memory(&engine).await;

    let engine_for_task = Arc::clone(&engine);
    let task = tokio::spawn(async move { run_check_with_config(&engine_for_task, config).await });
    fetch_started
        .await
        .expect("the scratch item was stored and fetch began");
    task.abort();
    cleanup_finished
        .await
        .expect("detached cleanup finishes after the check is cancelled");

    let namespace = {
        let calls = engine.calls.lock().unwrap();
        assert_eq!(calls.steps, ["store", "fetch", "forget_within"]);
        assert_eq!(calls.unscoped_forgets, 0);
        assert_eq!(calls.scoped_forgets.len(), 1);
        let namespace = calls.stored[0].1.clone();
        assert_eq!(calls.scoped_forgets[0].1, Reach::exact(namespace.clone()));
        namespace
    };

    assert_scratch_namespaces_are_empty(&engine, &[namespace]).await;
    let remaining = stored(&engine.inner, MetaFilter::default()).await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, real_id);
}

#[tokio::test]
async fn cancelling_an_ambiguous_store_cleans_up_using_the_precomputed_id() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let config = config_in(&tmp);
    let (probe, store_started, cleanup_finished) = ProbeEngine::pending_store();
    let engine = Arc::new(probe);
    let real_id = seed_real_memory(&engine).await;

    let engine_for_task = Arc::clone(&engine);
    let task = tokio::spawn(async move { run_check_with_config(&engine_for_task, config).await });
    store_started
        .await
        .expect("the store was accepted but its response is pending");
    task.abort();
    cleanup_finished
        .await
        .expect("detached cleanup finishes after the store is cancelled");

    let namespace = {
        let calls = engine.calls.lock().unwrap();
        assert_eq!(calls.steps, ["store", "forget_within"]);
        assert_eq!(calls.unscoped_forgets, 0);
        assert_eq!(calls.scoped_forgets.len(), 1);
        assert_eq!(calls.scoped_forgets[0].0, vec![calls.stored[0].0.clone()]);
        let namespace = calls.stored[0].1.clone();
        assert_eq!(calls.scoped_forgets[0].1, Reach::exact(namespace.clone()));
        namespace
    };

    assert_scratch_namespaces_are_empty(&engine, &[namespace]).await;
    let remaining = stored(&engine.inner, MetaFilter::default()).await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, real_id);
}

#[tokio::test]
async fn inspection_binding_rejects_disabled_and_unregistered_engines_without_fallback() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let mut config = config_in(&tmp);
    config.components.memory = false;
    assert!(matches!(
        crate::memory::engine::resolve_read_only_for_inspection(&config).await,
        Err(crate::memory::engine_registry::ReadOnlyEngineError::ComponentDisabled)
    ));

    config.components.memory = true;
    config.memory.engine = format!("doctor-unregistered-{}", uuid::Uuid::new_v4().simple());
    assert!(matches!(
        crate::memory::engine::resolve_read_only_for_inspection(&config).await,
        Err(crate::memory::engine_registry::ReadOnlyEngineError::UnsupportedEngine)
    ));
}

#[tokio::test]
async fn inspection_engine_factories_are_called_for_each_live_probe_without_caching() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let mut config = config_in(&tmp);
    let engine_id = format!("doctor-uncached-{}", uuid::Uuid::new_v4().simple());
    config.memory.engine = engine_id.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let factory: crate::memory::engine_registry::ReadOnlyEngineFactory = Arc::new(move |_| {
        factory_calls.fetch_add(1, Ordering::SeqCst);
        let engine: Arc<dyn MemoryEngine> = Arc::new(ReferenceEngine::new());
        Box::pin(async move { Ok(engine) })
    });
    let extensions = Extensions::new();
    crate::memory::engine_registry::register_read_only_engine_factory_in(
        &extensions,
        engine_id,
        factory,
    );
    let context = CoreContext::for_test_with_extensions(DomainSet::full(), None, extensions);

    CoreContext::scope(context, async {
        let first = crate::memory::engine::resolve_read_only_for_inspection(&config)
            .await
            .expect("first selected-config inspection bind");
        let second = crate::memory::engine::resolve_read_only_for_inspection(&config)
            .await
            .expect("second selected-config inspection bind");
        assert!(!Arc::ptr_eq(&first, &second));
    })
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
