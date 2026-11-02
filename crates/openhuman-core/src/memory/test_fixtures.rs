//! Shared fixtures for the memory unit tests: a config rooted in a temp dir,
//! and TinyMemory's in-memory reference engine bound to it.

use std::sync::Arc;

use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{ListRequest, MemoryEngine, MetaFilter};

use crate::config::Config;

/// A config whose workspace, action dir and credential store live in `tmp`.
/// The engine stays `tinyhumans` with no credential, so memory is off until a
/// test binds an engine.
pub(crate) fn config_in(tmp: &tempfile::TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    #[cfg(feature = "storage-sqlite")]
    crate::storage::local::use_persistent_default_for_test(&workspace);
    Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

/// Binds a fresh reference engine to `config`'s workspace and returns it.
pub(crate) fn bind_reference(config: &Config) -> Arc<ReferenceEngine> {
    let engine = Arc::new(ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    engine
}

/// Every item `engine` holds that matches `filter`.
pub(crate) async fn stored(
    engine: &ReferenceEngine,
    filter: MetaFilter,
) -> Vec<tinymemory_api::Hit> {
    engine
        .list(ListRequest {
            filter,
            limit: 100,
            cursor: None,
        })
        .await
        .expect("list")
        .items
}

/// An engine that refuses every operation with one error, the way the hosted
/// engine refuses a whole account (no credits, a rejected key, an outage).
/// It describes itself as the reference engine.
pub(crate) struct RefusingEngine {
    inner: ReferenceEngine,
    error: tinymemory_api::Error,
}

impl RefusingEngine {
    /// The hosted engine's refusal for an exhausted credit balance (HTTP 402).
    pub(crate) fn out_of_credits() -> Self {
        Self::with(tinymemory_api::Error::InsufficientCredits(
            "memory API fetch on api.example: the account has insufficient credits (HTTP 402)"
                .into(),
        ))
    }

    /// Refuses every operation with `error`.
    pub(crate) fn with(error: tinymemory_api::Error) -> Self {
        Self {
            inner: ReferenceEngine::new(),
            error,
        }
    }

    /// Binds a refusing engine to `config`'s workspace.
    pub(crate) fn bind(self, config: &Config) {
        crate::memory::engine::install_test_engine(&config.workspace_dir, Arc::new(self));
    }
}

#[async_trait::async_trait]
impl MemoryEngine for RefusingEngine {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }

    async fn recall(
        &self,
        _req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        Err(self.error.clone())
    }

    async fn fetch(
        &self,
        _req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        Err(self.error.clone())
    }

    async fn store(
        &self,
        _item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        Err(self.error.clone())
    }

    async fn forget(
        &self,
        _target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        Err(self.error.clone())
    }

    async fn list(&self, _req: ListRequest) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        Err(self.error.clone())
    }

    async fn export(
        &self,
        _req: ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ExportPage> {
        Err(self.error.clone())
    }

    async fn consolidate(
        &self,
        _req: tinymemory_api::ConsolidateRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ConsolidateReceipt> {
        Err(self.error.clone())
    }
}

/// The reference engine, recording which deletions it was asked for, and
/// optionally refusing to erase the way an engine without erasure does.
pub(crate) struct RecordingEngine {
    pub(crate) inner: ReferenceEngine,
    /// `"erase"` / `"forget"`, in call order.
    pub(crate) calls: std::sync::Mutex<Vec<&'static str>>,
    erase_unsupported: bool,
    fail_next_store: std::sync::atomic::AtomicBool,
    fail_next_forget: std::sync::atomic::AtomicBool,
    pause_next_store: std::sync::atomic::AtomicBool,
    store_paused: tokio::sync::Notify,
    release_store: tokio::sync::Notify,
}

impl RecordingEngine {
    /// Records and passes every call through.
    pub(crate) fn new() -> Self {
        Self {
            inner: ReferenceEngine::new(),
            calls: std::sync::Mutex::new(Vec::new()),
            erase_unsupported: false,
            fail_next_store: std::sync::atomic::AtomicBool::new(false),
            fail_next_forget: std::sync::atomic::AtomicBool::new(false),
            pause_next_store: std::sync::atomic::AtomicBool::new(false),
            store_paused: tokio::sync::Notify::new(),
            release_store: tokio::sync::Notify::new(),
        }
    }

    /// Records, and refuses every erase with `Unsupported`.
    pub(crate) fn without_erase() -> Self {
        Self {
            erase_unsupported: true,
            ..Self::new()
        }
    }

    /// Binds `engine` to `config`'s workspace.
    pub(crate) fn bind(engine: &Arc<Self>, config: &Config) {
        crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    }

    /// The recorded calls.
    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }

    /// Makes the next store fail, then resumes normal behavior.
    pub(crate) fn fail_next_store(&self) {
        self.fail_next_store
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Makes the next forget fail, then resumes normal behavior.
    pub(crate) fn fail_next_forget(&self) {
        self.fail_next_forget
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Pauses the next store until [`Self::release_paused_store`] is called.
    pub(crate) fn pause_next_store(&self) {
        self.pause_next_store
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Waits until the paused store has reached the barrier.
    pub(crate) async fn wait_for_paused_store(&self) {
        self.store_paused.notified().await;
    }

    /// Releases a store paused by [`Self::pause_next_store`].
    pub(crate) fn release_paused_store(&self) {
        self.release_store.notify_one();
    }
}

#[async_trait::async_trait]
impl MemoryEngine for RecordingEngine {
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
        self.inner.recall(req).await
    }

    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.inner.fetch(req).await
    }

    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        if self
            .pause_next_store
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.store_paused.notify_one();
            self.release_store.notified().await;
        }
        if self
            .fail_next_store
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(tinymemory_api::Error::Engine(
                "injected store failure".into(),
            ));
        }
        self.inner.store(item).await
    }

    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.calls.lock().unwrap().push("forget");
        if self
            .fail_next_forget
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(tinymemory_api::Error::Engine(
                "injected forget failure".into(),
            ));
        }
        self.inner.forget(target).await
    }

    async fn forget_within(
        &self,
        ids: Vec<tinymemory_api::ItemId>,
        reach: tinymemory_api::Reach,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.calls.lock().unwrap().push("forget_within");
        self.inner.forget_within(ids, reach).await
    }

    async fn list(&self, req: ListRequest) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }

    async fn export(&self, req: ListRequest) -> tinymemory_api::Result<tinymemory_api::ExportPage> {
        self.inner.export(req).await
    }

    async fn consolidate(
        &self,
        req: tinymemory_api::ConsolidateRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ConsolidateReceipt> {
        self.inner.consolidate(req).await
    }

    async fn erase(
        &self,
        req: tinymemory_api::EraseRequest,
    ) -> tinymemory_api::Result<tinymemory_api::EraseReport> {
        self.calls.lock().unwrap().push("erase");
        if self.erase_unsupported {
            return Err(tinymemory_api::Error::Unsupported("no erase".into()));
        }
        self.inner.erase(req).await
    }
}
