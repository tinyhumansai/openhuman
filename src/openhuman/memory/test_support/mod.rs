//! Test-only driver construction for handlers that read through a family the
//! null driver does not serve.
//!
//! Lives in a `test_support` directory because that is what the memory-guard
//! bypass scanner skips by path (`bypass_allowlist_tests::is_test_path`). The
//! alternative was an allowlist entry, and the allowlist's own rule is that it
//! may shrink but never grow — a test fixture is not the kind of bypass that
//! list exists to track.

use super::binding::install_for_test;
use crate::openhuman::memory::api::provider::MemoryProvider;
use std::sync::Arc;

/// Bind a fake driver that serves every optional family over a whole `Config`'s
/// workspace.
///
/// The shorthand for a test whose handler reads through a family the null
/// driver does not serve — `Chunks`, `Documents`, `Retrieval`.
/// `FixedDiagnostics` cannot serve those: it answers `Maintenance` and
/// delegates the rest to null.
///
/// # Why this is not an engine any more
///
/// It used to build a real `TinycortexProvider` over a temp workspace, and that
/// is what kept `tinycortex` and `tinymemory-core` — 133k lines — on this
/// crate's test critical path long after they left the product build
/// (openhuman#5560). The docstring justified it on the grounds that the
/// alternative was the bus, and a `dlopen`ed module is a process singleton that
/// hangs when a second test loads it.
///
/// That was a false choice: the third option is a driver that is neither the
/// engine nor the bus. `tinymemory-conformance` ships one, it is held to the
/// same contract as TinyCortex by `assert_provider`, and the engine is run
/// against those same assertions upstream — so what a test observes here is
/// contract behaviour rather than one engine's behaviour.
///
/// **What it deliberately will not do is filter, rank, or summarise.** A test
/// that needs those is asserting engine semantics, and upstream owns them; the
/// fake staying simple is what stops such a test from passing here against
/// nothing but the fake.
pub(crate) fn install_memory_driver_for_test(config: &crate::openhuman::config::Config) {
    let provider: Arc<dyn MemoryProvider> =
        Arc::new(tinymemory_conformance::RecordingProvider::new());
    install_for_test(&config.workspace_dir, &config.subsystems.memory, provider);
}

/// A [`Memory`] that stores nothing.
///
/// Seventeen test helpers used to obtain one by asking the engine's factory for
/// `backend: "none"` — an engine call whose entire purpose was to get back
/// something that does not store. The agent or session under test needs *a*
/// memory to be constructed with and never reads one back, so this is the same
/// behaviour without linking an engine to obtain it.
///
/// Deliberately not the conformance driver: that one retains, and a test that
/// asked for `"none"` was asking for the opposite. Swapping in a retaining
/// store would change what those tests exercise.
#[derive(Debug)]
pub(crate) struct NoopMemory;

#[async_trait::async_trait]
impl tinymemory_api::traits::Memory for NoopMemory {
    fn name(&self) -> &str {
        "none"
    }
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: tinymemory_api::types::MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: tinymemory_api::recall::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<tinymemory_api::types::MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn get(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> anyhow::Result<Option<tinymemory_api::types::MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&tinymemory_api::types::MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<tinymemory_api::types::MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<tinymemory_api::types::NamespaceSummary>> {
        Ok(Vec::new())
    }
    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn health_check(&self) -> bool {
        true
    }
}

/// The shorthand the `backend: "none"` call sites use.
pub(crate) fn noop_memory() -> Arc<dyn tinymemory_api::traits::Memory> {
    Arc::new(NoopMemory)
}

/// A [`Memory`] that keeps what it is given, for the handful of tests that
/// write through an agent and then read the store back.
///
/// [`NoopMemory`] cannot serve those, and the distinction is not cosmetic: two
/// `agent::` tests obtained a **sqlite-backed** store from the engine's factory
/// — `memory_store::create_memory(&MemoryConfig { backend: "sqlite", .. })` —
/// precisely because they assert on `count()` afterwards. Handing them a
/// no-op made one fail outright ("Expected at least 2 memory entries, got 0")
/// and, worse, made its sibling `auto_save_disabled_does_not_store` pass
/// **vacuously**: it asserts the store is empty, and a store that is always
/// empty agrees whether or not auto-save was actually disabled.
///
/// That is the whole reason this type exists rather than another `NoopMemory`
/// call site. A fixture that cannot fail is not a fixture.
///
/// Deliberately not sqlite, and deliberately not the engine's factory: what
/// those tests need is a store that retains, which is a `HashMap` behind a
/// lock. Nothing about them was ever about SQL.
///
/// [`Memory`]: tinymemory_api::traits::Memory
#[derive(Debug, Default)]
pub(crate) struct RetainingMemory {
    entries: std::sync::Mutex<Vec<tinymemory_api::types::MemoryEntry>>,
}

#[async_trait::async_trait]
impl tinymemory_api::traits::Memory for RetainingMemory {
    fn name(&self) -> &str {
        "retaining_test_memory"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: tinymemory_api::types::MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut entries = self.entries.lock().expect("entries lock");
        // Upsert on `(namespace, key)`, which is the contract's own rule for
        // the entry tier — a second write under one key must replace, not
        // accumulate, or a count assertion measures retries.
        if let Some(existing) = entries
            .iter_mut()
            .find(|e| e.namespace.as_deref() == Some(namespace) && e.key == key)
        {
            existing.content = content.to_string();
            return Ok(());
        }
        entries.push(tinymemory_api::types::MemoryEntry {
            id: format!("{namespace}:{key}"),
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            session_id: session_id.map(str::to_string),
            score: None,
            taint: Default::default(),
        });
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        _opts: tinymemory_api::recall::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<tinymemory_api::types::MemoryEntry>> {
        // Substring matching, not ranking. A test that needs relevance order
        // is asserting an engine's scoring model and belongs upstream.
        let entries = self.entries.lock().expect("entries lock");
        Ok(entries
            .iter()
            .filter(|e| e.content.contains(query))
            .take(limit)
            .cloned()
            .collect())
    }

    async fn get(
        &self,
        namespace: &str,
        key: &str,
    ) -> anyhow::Result<Option<tinymemory_api::types::MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .expect("entries lock")
            .iter()
            .find(|e| e.namespace.as_deref() == Some(namespace) && e.key == key)
            .cloned())
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&tinymemory_api::types::MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<tinymemory_api::types::MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .expect("entries lock")
            .iter()
            .filter(|e| namespace.is_none_or(|ns| e.namespace.as_deref() == Some(ns)))
            .filter(|e| category.is_none_or(|c| &e.category == c))
            .filter(|e| session_id.is_none_or(|s| e.session_id.as_deref() == Some(s)))
            .cloned()
            .collect())
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        let mut entries = self.entries.lock().expect("entries lock");
        let before = entries.len();
        entries.retain(|e| !(e.namespace.as_deref() == Some(namespace) && e.key == key));
        Ok(entries.len() != before)
    }

    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<tinymemory_api::types::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.entries.lock().expect("entries lock").len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

/// The shorthand for a test that writes through an agent and reads it back.
pub(crate) fn retaining_memory() -> Arc<dyn tinymemory_api::traits::Memory> {
    Arc::new(RetainingMemory::default())
}

/// Wait out any in-flight load of the memory module.
///
/// A module is loaded once per process, by whichever caller asks first, and
/// `modules::ops::state_of` reports `Loading` for the whole of it. Two correct
/// answers to that transient are indistinguishable from a regression:
/// `MemoryProvider::health` returns `degraded("the memory module is loading")`,
/// and a handler that reaches the driver answers "memory is still starting".
/// A test asserting a settled value therefore races whichever sibling triggered
/// the load — invisible when it runs alone, and lost about as often as not when
/// `cargo test --lib -- openhuman::memory` runs nine hundred of them in one
/// process (openhuman#6172).
///
/// Awaiting the resolution is what makes this race-free, where polling
/// `state_of` would only narrow the window: `Ready` and `Failed` are both
/// terminal — tinybus keeps a refused library mapped, so a resolution is never
/// retried — which means the state cannot return to `Loading` after this
/// returns. The wait is bounded because a caller that gives up leaves the
/// resolution running rather than cancelling it.
///
/// `Failed` is ignored: a host with no artifact for its platform resolves to
/// it, that is settled too, and what the caller asserts about a driver that
/// could not load is the caller's business. `StillLoading` is not ignored —
/// it is the one outcome that leaves the caller in exactly the state this
/// function exists to rule out, so it panics rather than handing back an
/// unsettled module and letting the caller fail on the transient it was
/// supposed to have waited out.
#[cfg(feature = "modules")]
pub(crate) async fn settle_memory_module() {
    use crate::openhuman::modules::ops::LoadError;

    match crate::openhuman::modules::ops::ensure_loaded_within(
        &crate::openhuman::memory::binding::test_module_config(),
        crate::openhuman::memory::binding::MODULE_ID,
        Some(std::time::Duration::from_secs(30)),
    )
    .await
    {
        Ok(()) | Err(LoadError::Failed(_)) => {}
        Err(LoadError::StillLoading) => panic!(
            "the memory module did not settle within 30s; every assertion after \
             this point would race its load"
        ),
    }
}

/// Without the `modules` feature nothing loads a module, so nothing can be
/// caught mid-load.
#[cfg(not(feature = "modules"))]
pub(crate) async fn settle_memory_module() {}
