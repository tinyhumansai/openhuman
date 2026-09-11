//! `MockMemory` — a storing [`Memory`] for the tool-memory and experience tests.
//!
//! This was one line, `pub use tinymemory_core::tool_memory::test_helpers;`, and
//! it is why four `#[cfg(test)]` modules kept the engine on this crate's test
//! critical path: `memory::tool_memory::{store, capture}`,
//! `agent::experience::{capture, store}` and
//! `agent::tinyagents::host::experience_store` all bind an
//! `Arc<dyn Memory>` and write rows through it.
//!
//! Ported rather than deleted (openhuman#6161). Nothing about a `HashMap`
//! behind a mutex needed an engine — it was upstream only because that is where
//! it happened to be written, and importing it cost 133k lines of TinyCortex
//! and tinymemory-core for the privilege.
//!
//! Behaviour is the upstream fixture's, deliberately including its omissions:
//! `recall` answers empty and `list` ignores `category` and `session_id`. The
//! tests that use it assert the store/get/forget path, and widening the fixture
//! would change what they exercise.
//!
//! It lives in a `test_support/` **directory** because both memory ratchets skip
//! by path — `is_test_path` matches a path *component* named `test_support`, not
//! a file stem.
//!
//! `parking_lot::Mutex` rather than `std`'s, matching the upstream fixture: the
//! tests reach `memory.entries.lock()` directly, and a `std` mutex would make
//! every one of those call sites grow an `unwrap`.

use parking_lot::Mutex;
use std::collections::HashMap;

use async_trait::async_trait;
use tinymemory_api::recall::RecallOpts;
use tinymemory_api::traits::Memory;
use tinymemory_api::types::{MemoryCategory, MemoryEntry, NamespaceSummary};

/// The public path callers use is unchanged: `tool_memory::test_helpers::MockMemory`.
pub mod test_helpers {
    pub use super::MockMemory;
}

/// Minimal in-memory [`Memory`] backend for unit tests.
///
/// Stores entries in a `HashMap` keyed by `(namespace, key)`. Methods the
/// store/capture tests do not need are no-ops.
#[derive(Default)]
pub struct MockMemory {
    /// The rows, keyed the way the contract upserts them.
    pub entries: Mutex<HashMap<(String, String), MemoryEntry>>,
}

impl MockMemory {
    fn rows(&self) -> parking_lot::MutexGuard<'_, HashMap<(String, String), MemoryEntry>> {
        self.entries.lock()
    }
}

#[async_trait]
impl Memory for MockMemory {
    fn name(&self) -> &str {
        "mock"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.rows().insert(
            (namespace.to_string(), key.to_string()),
            MemoryEntry {
                id: format!("{namespace}/{key}"),
                key: key.to_string(),
                content: content.to_string(),
                namespace: Some(namespace.to_string()),
                category,
                timestamp: "now".into(),
                session_id: session_id.map(str::to_string),
                score: None,
                taint: Default::default(),
            },
        );
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(self
            .rows()
            .get(&(namespace.to_string(), key.to_string()))
            .cloned())
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let lock = self.rows();
        Ok(match namespace {
            Some(ns) => lock
                .iter()
                .filter(|((n, _), _)| n == ns)
                .map(|(_, v)| v.clone())
                .collect(),
            None => lock.values().cloned().collect(),
        })
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        Ok(self
            .rows()
            .remove(&(namespace.to_string(), key.to_string()))
            .is_some())
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (ns, _) in self.rows().keys() {
            *counts.entry(ns.clone()).or_default() += 1;
        }
        Ok(counts
            .into_iter()
            .map(|(namespace, count)| NamespaceSummary {
                namespace,
                count,
                last_updated: None,
            })
            .collect())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.rows().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}
