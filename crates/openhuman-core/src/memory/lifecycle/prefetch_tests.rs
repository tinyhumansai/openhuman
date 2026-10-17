use super::*;

use crate::memory::scope::MemoryIdentity;
use crate::memory::test_fixtures::config_in;

const MAX_PENDING: usize = 16;

static TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

fn pack(text: &str) -> TurnPack {
    TurnPack {
        markdown: text.into(),
        tokens: 1,
        refs: vec![],
        engine: "test".into(),
        citations: vec![],
        refusal: None,
    }
}

async fn drain(cache: &Cache, key: &Key) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !cache.0.is_idle(key) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("worker completed");
}

#[tokio::test]
async fn held_refresh_returns_immediately_and_later_turn_reads_completed_context() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = MemoryIdentity::agent("a").resolve(&config);
    let key = Key::new(&config, &identity, "t", false);
    let cache = Arc::new(Cache::default());
    let release = Arc::new(tokio::sync::Notify::new());
    let held = release.clone();
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async move {
            held.notified().await;
            Some(pack("later result"))
        }),
    );
    assert!(
        completed(&cache, &key).is_none(),
        "cold refresh never waits"
    );
    release.notify_one();
    drain(&cache, &key).await;
    assert_eq!(completed(&cache, &key).unwrap().markdown, "later result");
}

#[tokio::test]
async fn identity_config_thread_workspace_and_context_are_isolated() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = MemoryIdentity::agent("a").resolve(&config);
    let key = Key::new(&config, &identity, "t", false);
    let cache = Arc::new(Cache::default());
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async { Some(pack("private")) }),
    );
    drain(&cache, &key).await;
    let mut changed = config.clone();
    changed.memory.recall.brain_limit += 1;
    let mut omitted = identity.clone();
    omitted.recall = false;
    let mut other_root = identity.clone();
    other_root.layout = tinymemory_tools::MemoryLayout::new("team:other".parse().unwrap()).unwrap();
    let mut workspace = config.clone();
    workspace.workspace_dir = tmp.path().join("other");
    for other in [
        Key::new(&changed, &identity, "t", false),
        Key::new(&config, &omitted, "t", false),
        Key::new(&config, &other_root, "t", false),
        Key::new(&config, &identity, "other", false),
        Key::new(&workspace, &identity, "t", false),
    ] {
        assert!(completed(&cache, &other).is_none());
    }
    assert!(completed(&Cache::default(), &key).is_none());
}

#[tokio::test]
async fn invalidation_rejects_late_results_and_preserves_bounded_pending_logs() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = MemoryIdentity::agent("a").resolve(&config);
    let key = Key::new(&config, &identity, "t", false);
    let cache = Arc::new(Cache::default());
    CACHES.lock().unwrap().push(Arc::downgrade(&cache));
    let release = Arc::new(tokio::sync::Notify::new());
    let started = Arc::new(tokio::sync::Notify::new());
    let held = release.clone();
    let starting = started.clone();
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async move {
            starting.notify_one();
            held.notified().await;
            Some(pack("forgotten"))
        }),
    );
    started.notified().await;
    let logs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..MAX_PENDING + 10 {
        let logs = logs.clone();
        enqueue(
            cache.clone(),
            key.clone(),
            Box::pin(async move {
                logs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                None
            }),
        );
    }
    invalidate(&config);
    release.notify_one();
    drain(&cache, &key).await;
    assert!(completed(&cache, &key).is_none());
    assert_eq!(logs.load(std::sync::atomic::Ordering::SeqCst), MAX_PENDING);
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async { Some(pack("fresh")) }),
    );
    drain(&cache, &key).await;
    assert_eq!(completed(&cache, &key).unwrap().markdown, "fresh");
}

#[tokio::test]
async fn expiration_failure_and_oversized_packs_are_not_reused() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = MemoryIdentity::agent("a").resolve(&config);
    let key = Key::new(&config, &identity, "t", false);
    let cache = Arc::new(Cache::default());
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async { Some(pack("good")) }),
    );
    drain(&cache, &key).await;
    assert_eq!(completed(&cache, &key).unwrap().markdown, "good");
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async { Some(pack(&"x".repeat(MAX_PACK_BYTES + 1))) }),
    );
    drain(&cache, &key).await;
    assert!(completed(&cache, &key).is_none());
    enqueue(cache.clone(), key.clone(), Box::pin(async { None }));
    drain(&cache, &key).await;
    assert!(completed(&cache, &key).is_none());
}

#[tokio::test]
async fn credential_invalidation_removes_ready_and_rejects_late_inflight_results() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = MemoryIdentity::agent("a").resolve(&config);
    let key = Key::new(&config, &identity, "t", false);
    let cache = Arc::new(Cache::default());
    CACHES.lock().unwrap().push(Arc::downgrade(&cache));
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async { Some(pack("private")) }),
    );
    drain(&cache, &key).await;
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let starting = started.clone();
    let held = release.clone();
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async move {
            starting.notify_one();
            held.notified().await;
            Some(pack("old credentials"))
        }),
    );
    started.notified().await;
    assert_eq!(completed(&cache, &key).unwrap().markdown, "private");
    invalidate_all();
    assert!(completed(&cache, &key).is_none());
    enqueue(
        cache.clone(),
        key.clone(),
        Box::pin(async { Some(pack("new credentials")) }),
    );
    drain(&cache, &key).await;
    release.notify_one();
    tokio::task::yield_now().await;
    assert_eq!(completed(&cache, &key).unwrap().markdown, "new credentials");
}

struct HeldEngine {
    inner: tinymemory_api::conformance::ReferenceEngine,
    release: Arc<tokio::sync::Semaphore>,
}
#[async_trait::async_trait]
impl tinymemory_api::MemoryEngine for HeldEngine {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }

    async fn recall(
        &self,
        req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        let _permit = self.release.acquire().await.unwrap();
        self.inner.recall(req).await
    }

    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        let _permit = self.release.acquire().await.unwrap();
        self.inner.fetch(req).await
    }

    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        let _permit = self.release.acquire().await.unwrap();
        self.inner.store(item).await
    }

    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        let _permit = self.release.acquire().await.unwrap();
        self.inner.forget(target).await
    }

    async fn list(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        let _permit = self.release.acquire().await.unwrap();
        self.inner.list(req).await
    }

    async fn export(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ExportPage> {
        let _permit = self.release.acquire().await.unwrap();
        self.inner.export(req).await
    }

    async fn consolidate(
        &self,
        req: tinymemory_api::ConsolidateRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ConsolidateReceipt> {
        let _permit = self.release.acquire().await.unwrap();
        self.inner.consolidate(req).await
    }
}

#[tokio::test]
async fn slow_engine_results_survive_the_manual_pre_turn_cutoff() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.recall.pre_turn_timeout_ms = 1;
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let engine = Arc::new(HeldEngine {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        release: release.clone(),
    });
    use tinymemory_api::MemoryEngine;
    engine
        .inner
        .store(tinymemory_api::StoreItem::learning(
            "Favourite colour is teal",
            tinymemory_api::LearningKind::Preference,
            0.9,
            tinymemory_api::MemoryMeta::default(),
        ))
        .await
        .unwrap();
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    let identity = MemoryIdentity::agent("a").resolve(&config);
    let input = |index| PreTurnInput {
        thread_id: "slow".into(),
        turn_index: index,
        user_text: "what is my favourite colour?".into(),
        in_prompt_from: 0,
        at: chrono::Utc::now(),
        resumed_after_compaction: false,
        observed_actor: None,
    };
    let config = Arc::new(config);
    assert!(pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        config.clone(),
        identity.clone(),
        input(0),
        None
    )
    .is_none());
    // Even after the old one-millisecond cutoff, the pack remains owned by
    // the background worker instead of being discarded by the live turn.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let cache = cache(&crate::core::runtime::DispatchCtx::ambient());
    let key = Key::new(&config, &identity, "slow", false);
    assert!(completed(&cache, &key).is_none());
    release.add_permits(100);
    drain(&cache, &key).await;
    let pack = pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        config.clone(),
        identity.clone(),
        input(2),
        None,
    )
    .expect("next turn uses completed slow lookup");
    assert!(pack.markdown.contains("teal"));
    assert!(pack.markdown.contains("earlier turn"));
    drain(&cache, &key).await;
    let mut omitted = identity;
    omitted.recall = false;
    assert!(pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        config.clone(),
        omitted.clone(),
        input(4),
        None
    )
    .is_none());
    drain(&cache, &Key::new(&config, &omitted, "slow", false)).await;
    let turns = crate::memory::test_fixtures::stored(
        &engine.inner,
        tinymemory_api::MetaFilter::kinds([tinymemory_api::ItemKind::Conversation]),
    )
    .await;
    assert_eq!(turns.len(), 3, "omitted recall still logs its user turn");
}

#[tokio::test]
async fn queued_work_keeps_the_callers_core_context() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let ctx = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(config.workspace_dir.clone()),
    );
    crate::core::runtime::CoreContext::scope(ctx.clone(), async {
        let identity = MemoryIdentity::agent("scoped-agent").resolve(&config);
        let key = Key::new(&config, &identity, "t", false);
        let cache = cache(&crate::core::runtime::DispatchCtx::ambient());
        let expected = ctx.clone();
        enqueue(
            cache.clone(),
            key.clone(),
            Box::pin(
                crate::core::runtime::DispatchCtx::ambient().enter(async move {
                    assert!(Arc::ptr_eq(
                        &crate::core::runtime::CoreContext::scoped().unwrap(),
                        &expected
                    ));
                    Some(pack("scoped"))
                }),
            ),
        );
        drain(&cache, &key).await;
        assert_eq!(completed(&cache, &key).unwrap().markdown, "scoped");
    })
    .await;
}

#[test]
fn runtime_shutdown_releases_worker_and_pending_work() {
    let _guard = TEST_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = MemoryIdentity::agent("a").resolve(&config);
    let key = Key::new(&config, &identity, "t", false);
    let cache = Arc::new(Cache::default());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        enqueue(cache.clone(), key.clone(), Box::pin(std::future::pending()));
        enqueue(
            cache.clone(),
            key.clone(),
            Box::pin(async { Some(pack("queued")) }),
        );
        tokio::task::yield_now().await;
        assert!(!cache.0.is_idle(&key));
    });
    drop(runtime);
    assert!(cache.0.is_idle(&key));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        enqueue(
            cache.clone(),
            key.clone(),
            Box::pin(async { Some(pack("later")) }),
        );
        drain(&cache, &key).await;
        assert_eq!(completed(&cache, &key).unwrap().markdown, "later");
    });
}

#[tokio::test]
async fn an_erase_retires_queued_automatic_work_and_allows_later_work() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::memory::test_fixtures::config_in(&tmp);
    let identity = crate::memory::scope::MemoryIdentity::agent("erase-prefetch").resolve(&config);
    let key = Key::new(&config, &identity, "erase-prefetch", false);
    let cache = Arc::new(Cache::default());
    let earlier = Arc::new(AtomicUsize::new(0));
    let later = Arc::new(AtomicUsize::new(0));
    let old_count = earlier.clone();
    enqueue(
        cache.clone(),
        key.clone(),
        guard_turn_work(
            &config,
            Box::pin(async move {
                old_count.fetch_add(1, Ordering::SeqCst);
                Some(pack("earlier automatic log"))
            }),
        ),
    );
    // The current-thread executor has not polled the earlier job. Erasure
    // advances the owner fence synchronously before its first suspension.
    let fence = crate::memory::tool_writes::fence_and_clear(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
    )
    .await
    .unwrap();
    let new_count = later.clone();
    enqueue(
        cache.clone(),
        key.clone(),
        guard_turn_work(
            &config,
            Box::pin(async move {
                new_count.fetch_add(1, Ordering::SeqCst);
                Some(pack("later automatic log"))
            }),
        ),
    );
    drop(fence);
    drain(&cache, &key).await;
    assert_eq!(
        earlier.load(Ordering::SeqCst),
        0,
        "old automatic logs must never poll after erase begins"
    );
    assert_eq!(later.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn erase_waits_for_current_automatic_work_before_returning_its_barrier() {
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::memory::test_fixtures::config_in(&tmp);
    let identity = crate::memory::scope::MemoryIdentity::agent("erase-active").resolve(&config);
    let key = Key::new(&config, &identity, "erase-active", false);
    let cache = Arc::new(Cache::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let began = started.clone();
    let done = release.clone();
    enqueue(
        cache.clone(),
        key.clone(),
        guard_turn_work(
            &config,
            Box::pin(async move {
                began.notify_one();
                done.notified().await;
                Some(pack("current automatic log"))
            }),
        ),
    );
    started.notified().await;
    let erase_config = config.clone();
    let mut fencing = tokio::spawn(async move {
        crate::memory::tool_writes::fence_and_clear(
            &crate::core::runtime::DispatchCtx::ambient(),
            &erase_config,
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_secs(1), &mut fencing)
            .await
            .is_err(),
        "erase must wait for the active automatic mutation"
    );
    release.notify_one();
    let fence = fencing.await.unwrap().unwrap();
    drop(fence);
    drain(&cache, &key).await;
}
