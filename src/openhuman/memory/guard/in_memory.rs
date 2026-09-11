//! An in-memory [`MemoryProvider`] that actually stores things.
//!
//! # Why this exists
//!
//! The module port keeps meeting the same test problem. A consumer used to be
//! handed an `Arc<dyn Memory>` and tests handed it a real `UnifiedMemory` over
//! a temp dir, then asserted a genuine round trip: write, read back, prune.
//! Converting the consumer to the guard breaks those tests, and the cheap
//! answer — `#[ignore]` behind `OPENHUMAN_MODULE_PATH` — pays for each
//! conversion with real coverage.
//!
//! [`super::test_support::RecordingProvider`] cannot stand in: it records calls
//! and answers empty, which proves a call was *made* but never that the data
//! came back. Round-trip assertions need storage.
//!
//! So this is storage: a `HashMap` keyed by `(namespace, key)` behind a mutex,
//! implementing the mandatory three so it can be wrapped in a real
//! [`MemoryGuard`](super::MemoryGuard) and dropped in wherever a consumer now
//! wants a guard.
//!
//! # It is deliberately not `#[cfg(test)]`
//!
//! Integration tests under `tests/` link the library compiled without
//! `cfg(test)`, so a test-gated helper is invisible to them — the trap
//! `ProfileStore::for_tests` documents and this port has already fallen into
//! once. `#[doc(hidden)]` keeps it off the public docs instead.
//!
//! # What it does not pretend to be
//!
//! `recall` is a substring match over content, not a ranked hybrid search. That
//! is enough for "did the write land and come back", which is what these tests
//! assert; it is **not** enough to test ranking, and a test about ordering
//! should use the real engine rather than this.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::openhuman::memory::api::capabilities::Capabilities;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::types::SourceScope;
use crate::openhuman::memory::api::provider::types::{ExportPage, ExportRecord, ImportOutcome};
use crate::openhuman::memory::api::provider::{
    MemoryCore, MemoryPortability, MemoryProvider, MemoryRecall,
};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary,
};

/// Entries held in memory, keyed by `(namespace, key)`.
#[derive(Default)]
pub struct InMemoryProvider {
    entries: Mutex<HashMap<(String, String), MemoryEntry>>,
}

impl InMemoryProvider {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many entries are stored, for assertions about pruning.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    /// Whether the store holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }
}

#[async_trait]
impl MemoryCore for InMemoryProvider {
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        let entry = MemoryEntry {
            id: format!("{namespace}/{key}"),
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            // Monotonic enough for "oldest first" pruning assertions, and
            // stable to render.
            timestamp: chrono::Utc::now().to_rfc3339(),
            session_id: session_id.map(str::to_string),
            score: None,
            taint,
        };
        self.entries
            .lock()
            .insert((namespace.to_string(), key.to_string()), entry);
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        Ok(self
            .entries
            .lock()
            .get(&(namespace.to_string(), key.to_string()))
            .cloned())
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        Ok(self
            .entries
            .lock()
            .remove(&(namespace.to_string(), key.to_string()))
            .is_some())
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let entries = self.entries.lock();
        let mut out: Vec<MemoryEntry> = entries
            .values()
            .filter(|e| namespace.is_none_or(|ns| e.namespace.as_deref() == Some(ns)))
            .filter(|e| category.is_none_or(|c| &e.category == c))
            .filter(|e| session_id.is_none_or(|s| e.session_id.as_deref() == Some(s)))
            .cloned()
            .collect();
        // Deterministic order — a `HashMap`'s iteration order varies per
        // process and would make an otherwise-identical assertion flaky.
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        let entries = self.entries.lock();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for entry in entries.values() {
            if let Some(ns) = &entry.namespace {
                *counts.entry(ns.clone()).or_default() += 1;
            }
        }
        let mut out: Vec<NamespaceSummary> = counts
            .into_iter()
            .map(|(namespace, count)| NamespaceSummary {
                namespace,
                count,
                last_updated: None,
            })
            .collect();
        out.sort_by(|a, b| a.namespace.cmp(&b.namespace));
        Ok(out)
    }
}

#[async_trait]
impl MemoryRecall for InMemoryProvider {
    /// Substring match, not ranking — see the module docs.
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &OwnedRecallOpts,
        _scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let needle = query.to_lowercase();
        let entries = self.entries.lock();
        let mut out: Vec<MemoryEntry> = entries
            .values()
            .filter(|e| {
                opts.namespace
                    .as_deref()
                    .is_none_or(|ns| e.namespace.as_deref() == Some(ns))
            })
            .filter(|e| e.content.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out.truncate(limit);
        Ok(out)
    }
}

#[async_trait]
impl MemoryPortability for InMemoryProvider {
    async fn export_page(
        &self,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        Err(MemoryError::Other(anyhow::anyhow!(
            "InMemoryProvider does not implement export"
        )))
    }

    async fn import_records(
        &self,
        _records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        Err(MemoryError::Other(anyhow::anyhow!(
            "InMemoryProvider does not implement import"
        )))
    }
}

#[async_trait]
impl MemoryProvider for InMemoryProvider {
    fn driver_id(&self) -> &str {
        "in-memory"
    }

    /// Only the mandatory three. Advertising more would fail
    /// `audit_provider`, since no optional accessor is overridden.
    fn capabilities(&self) -> Capabilities {
        Capabilities::mandatory()
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
    }
}

/// A real [`MemoryGuard`](super::MemoryGuard) over a fresh in-memory store,
/// plus the store itself for direct assertions.
#[must_use]
pub fn guarded_in_memory() -> (Arc<InMemoryProvider>, Arc<super::MemoryGuard>) {
    let provider = Arc::new(InMemoryProvider::new());
    let guard = guard_over(Arc::clone(&provider) as Arc<dyn MemoryProvider>);
    (provider, guard)
}

/// Wrap any provider in a real [`MemoryGuard`](super::MemoryGuard) at the
/// trusted tier.
///
/// Split out of [`guarded_in_memory`] so a test that needs an optional family
/// — retrieval, say — can supply its own provider and still be exercised
/// through the same policy decorator production uses, rather than calling the
/// provider directly and skipping the guard entirely.
#[must_use]
pub fn guard_over(provider: Arc<dyn MemoryProvider>) -> Arc<super::MemoryGuard> {
    let policy = Arc::new(super::GuardPolicy::new(
        "in-memory",
        crate::core::subsystem::DriverClass::Embedded,
        crate::openhuman::config::schema::MemoryHooksConfig::default(),
        super::policy::TRUSTED,
    ));
    Arc::new(super::MemoryGuard::new(provider, policy))
}

/// A provider whose `recall` answers with a fixed entry list, whatever the
/// query.
///
/// # Why this is not [`InMemoryProvider`]
///
/// That one substring-matches, which is right for a round-trip test and wrong
/// for the several channel/context tests that script a specific result set —
/// scored entries, an over-long entry, ten entries to overflow a budget — and
/// assert on what the *caller* does with it. Those tests are about rendering
/// and filtering downstream of recall, so recall itself has to be a constant.
///
/// Everything else is inert: writes are accepted and dropped, reads answer
/// empty. A test needing real storage wants [`InMemoryProvider`].
pub struct FixedRecallProvider {
    entries: Vec<MemoryEntry>,
}

impl FixedRecallProvider {
    #[must_use]
    pub fn new(entries: Vec<MemoryEntry>) -> Self {
        Self { entries }
    }

    /// The provider wrapped in a real guard, ready to drop into a context.
    #[must_use]
    pub fn guarded(entries: Vec<MemoryEntry>) -> Arc<super::MemoryGuard> {
        guard_over(Arc::new(Self::new(entries)) as Arc<dyn MemoryProvider>)
    }
}

#[async_trait]
impl MemoryCore for FixedRecallProvider {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
        _taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        Ok(None)
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool, MemoryError> {
        Ok(false)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        Ok(Vec::new())
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl MemoryRecall for FixedRecallProvider {
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: &OwnedRecallOpts,
        _scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        Ok(self.entries.clone())
    }
}

#[async_trait]
impl MemoryPortability for FixedRecallProvider {
    async fn export_page(
        &self,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        Err(MemoryError::Other(anyhow::anyhow!(
            "FixedRecallProvider does not implement export"
        )))
    }

    async fn import_records(
        &self,
        _records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        Err(MemoryError::Other(anyhow::anyhow!(
            "FixedRecallProvider does not implement import"
        )))
    }
}

#[async_trait]
impl MemoryProvider for FixedRecallProvider {
    fn driver_id(&self) -> &str {
        "fixed-recall"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::mandatory()
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
    }
}
