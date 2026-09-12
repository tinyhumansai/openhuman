use super::*;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use async_trait::async_trait;

struct MockMemory {
    entries: Vec<MemoryEntry>,
    cross_chat: Vec<MemoryEntry>,
}

impl MockMemory {
    fn new(entries: Vec<MemoryEntry>) -> Self {
        Self {
            entries,
            cross_chat: Vec::new(),
        }
    }
}

#[async_trait]
impl Memory for MockMemory {
    fn name(&self) -> &str {
        "mock"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        opts: crate::openhuman::memory::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        if opts.cross_session {
            return Ok(self.cross_chat.clone());
        }
        Ok(self.entries.clone())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.entries.len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn entry(key: &str, content: &str, score: Option<f64>) -> MemoryEntry {
    MemoryEntry {
        id: format!("id-{key}"),
        key: key.to_string(),
        content: content.to_string(),
        namespace: Some("test".to_string()),
        category: MemoryCategory::Conversation,
        timestamp: "2026-04-22T00:00:00Z".to_string(),
        session_id: None,
        score,
        taint: Default::default(),
    }
}

#[tokio::test]
async fn collect_recall_citations_filters_and_truncates_entries() {
    let mem = MockMemory::new(vec![
        entry("keep", "useful context", Some(0.9)),
        entry("drop", "too weak", Some(0.1)),
        entry("long", &"x".repeat(600), Some(0.8)),
    ]);

    let citations = collect_recall_citations(&mem, "hello", 5, 0.4)
        .await
        .expect("citation collection should succeed");
    assert_eq!(citations.len(), 2);
    assert_eq!(citations[0].key, "keep");
    assert_eq!(citations[1].key, "long");
    assert!(citations[1].snippet.ends_with("..."));
}

// ── Cross-chat context (#1505) ───────────────────────────────────────
