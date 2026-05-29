//! Shared test infrastructure for the tool-scoped memory layer.
//!
//! Only compiled under `#[cfg(test)]`.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};

/// Minimal in-memory [`Memory`] backend for unit tests.
///
/// Stores entries in a `HashMap` keyed by `(namespace, key)`.  All methods
/// that are not needed by the store/capture tests are no-ops.
#[derive(Default)]
pub struct MockMemory {
    pub entries: Mutex<HashMap<(String, String), MemoryEntry>>,
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
        self.entries.lock().insert(
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
            .entries
            .lock()
            .get(&(namespace.to_string(), key.to_string()))
            .cloned())
    }
    async fn list(
        &self,
        namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let lock = self.entries.lock();
        Ok(match namespace {
            Some(ns) => lock
                .iter()
                .filter(|((n, _), _)| n == ns)
                .map(|(_, v)| v.clone())
                .collect(),
            None => lock.iter().map(|(_, v)| v.clone()).collect(),
        })
    }
    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        Ok(self
            .entries
            .lock()
            .remove(&(namespace.to_string(), key.to_string()))
            .is_some())
    }
    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for ((ns, _), _) in self.entries.lock().iter() {
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
        Ok(self.entries.lock().len())
    }
    async fn health_check(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_memory_store_get_list_and_count_roundtrip() {
        let memory = MockMemory::default();
        memory
            .store(
                "tool-bash",
                "rule/1",
                "always dry run first",
                MemoryCategory::Custom("tool_memory".into()),
                Some("session-1"),
            )
            .await
            .unwrap();
        memory
            .store(
                "tool-web",
                "rule/2",
                "cite sources",
                MemoryCategory::Conversation,
                None,
            )
            .await
            .unwrap();

        let got = memory.get("tool-bash", "rule/1").await.unwrap().unwrap();
        assert_eq!(got.id, "tool-bash/rule/1");
        assert_eq!(got.content, "always dry run first");
        assert_eq!(got.namespace.as_deref(), Some("tool-bash"));
        assert_eq!(got.session_id.as_deref(), Some("session-1"));

        let scoped = memory.list(Some("tool-bash"), None, None).await.unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].key, "rule/1");

        let all = memory.list(None, None, None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(memory.count().await.unwrap(), 2);
        assert!(memory.health_check().await);
        assert_eq!(memory.name(), "mock");

        // The mock intentionally ignores category/session filters so tool
        // tests can focus on caller behavior instead of backend indexing.
        let filtered = memory
            .list(
                Some("tool-bash"),
                Some(&MemoryCategory::Core),
                Some("different-session"),
            )
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].key, "rule/1");
    }

    #[tokio::test]
    async fn mock_memory_forget_and_namespace_summaries_track_entries() {
        let memory = MockMemory::default();
        memory
            .store("tool-bash", "rule/1", "first", MemoryCategory::Core, None)
            .await
            .unwrap();
        memory
            .store("tool-bash", "rule/2", "second", MemoryCategory::Daily, None)
            .await
            .unwrap();
        memory
            .store(
                "tool-web",
                "rule/3",
                "third",
                MemoryCategory::Conversation,
                None,
            )
            .await
            .unwrap();

        let mut summaries = memory.namespace_summaries().await.unwrap();
        summaries.sort_by(|a, b| a.namespace.cmp(&b.namespace));
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].namespace, "tool-bash");
        assert_eq!(summaries[0].count, 2);
        assert_eq!(summaries[1].namespace, "tool-web");
        assert_eq!(summaries[1].count, 1);

        assert!(memory.forget("tool-bash", "rule/1").await.unwrap());
        assert!(!memory.forget("tool-bash", "missing").await.unwrap());

        let remaining = memory.list(Some("tool-bash"), None, None).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].key, "rule/2");
    }

    #[tokio::test]
    async fn mock_memory_recall_is_empty_noop() {
        let memory = MockMemory::default();
        let recalled = memory
            .recall("anything", 5, RecallOpts::default())
            .await
            .unwrap();
        assert!(recalled.is_empty());
    }

    #[tokio::test]
    async fn mock_memory_empty_state_helpers_return_empty_values() {
        let memory = MockMemory::default();
        assert!(memory.get("missing", "rule").await.unwrap().is_none());
        assert!(memory
            .list(Some("missing"), None, None)
            .await
            .unwrap()
            .is_empty());
        assert!(memory.namespace_summaries().await.unwrap().is_empty());
        assert_eq!(memory.count().await.unwrap(), 0);
    }
}
