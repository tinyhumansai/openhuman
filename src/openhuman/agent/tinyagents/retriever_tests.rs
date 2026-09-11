use super::*;
use async_trait::async_trait;
use tinyagents_harness::events::{EventListener, EventRecord};

use crate::openhuman::memory::{MemoryCategory, MemoryEntry, NamespaceSummary};

struct StubMemory {
    entries: Vec<MemoryEntry>,
}

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "stub"
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
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
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
    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }
    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.entries.len())
    }
    async fn health_check(&self) -> bool {
        true
    }
}

fn entry(id: &str, key: &str, namespace: Option<&str>, score: Option<f64>) -> MemoryEntry {
    MemoryEntry {
        id: id.into(),
        key: key.into(),
        content: "content".into(),
        namespace: namespace.map(str::to_string),
        category: MemoryCategory::Conversation,
        timestamp: "now".into(),
        session_id: None,
        score,
        taint: Default::default(),
    }
}

/// A listener that counts `MemoryLoaded` records it observes.
struct MemoryLoadedCounter {
    count: Arc<std::sync::atomic::AtomicUsize>,
}

impl EventListener for MemoryLoadedCounter {
    fn on_event(&self, record: &EventRecord) {
        if matches!(record.event, AgentEvent::MemoryLoaded) {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[test]
fn path_scope_prefers_namespace_then_id_prefix_then_global() {
    assert_eq!(
        derive_path_scope(&entry("id-1", "k", Some("notion:conn-1"), None)),
        "notion:conn-1"
    );
    assert_eq!(
        derive_path_scope(&entry("episodic-cross:42", "k", None, None)),
        "episodic-cross"
    );
    assert_eq!(
        derive_path_scope(&entry("plainid", "k", None, None)),
        "global"
    );
}

#[test]
fn scored_doc_projection_carries_path_scope_into_metadata() {
    let doc = entry_to_scored_doc(&entry("id-1", "task", Some("ns-a"), Some(0.75)));
    assert_eq!(doc.id, "id-1");
    assert!((doc.score - 0.75).abs() < 1e-6);
    assert_eq!(doc.metadata["path_scope"], "ns-a");
    assert_eq!(doc.metadata["key"], "task");
}

#[test]
fn dedupe_collapses_by_id_and_preserves_order() {
    let docs = vec![
        entry_to_scored_doc(&entry("dup", "a", None, Some(0.9))),
        entry_to_scored_doc(&entry("other", "b", None, Some(0.8))),
        entry_to_scored_doc(&entry("dup", "a2", None, Some(0.1))),
    ];
    let out = dedupe_scored_docs(docs);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, "dup");
    assert_eq!(out[0].metadata["key"], "a"); // first occurrence wins
    assert_eq!(out[1].id, "other");
}

#[tokio::test]
async fn facade_returns_entries_unchanged_for_unique_ids() {
    let entries = vec![
        entry("id-1", "task", Some("ns"), Some(0.9)),
        entry("id-2", "low", Some("ns"), Some(0.1)),
    ];
    let mem = StubMemory {
        entries: entries.clone(),
    };
    let out = recall_through_facade(&mem, "q", 5, RecallOpts::default())
        .await
        .expect("facade recall");
    // Byte-identical passthrough: same ids, same order, same count.
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, "id-1");
    assert_eq!(out[1].id, "id-2");
}

#[tokio::test]
async fn facade_emits_memory_loaded_when_entries_present() {
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    memory_event_sink().subscribe(Arc::new(MemoryLoadedCounter {
        count: counter.clone(),
    }));

    let mem = StubMemory {
        entries: vec![entry("id-1", "task", Some("ns"), Some(0.9))],
    };
    let _ = recall_through_facade(&mem, "q", 5, RecallOpts::default())
        .await
        .expect("facade recall");
    assert!(
        counter.load(std::sync::atomic::Ordering::SeqCst) >= 1,
        "MemoryLoaded must be emitted when context loads"
    );
}

#[tokio::test]
async fn build_retriever_indexes_and_retrieves_scored_docs() {
    // Deterministic stub provider so the engine seam is exercised offline.
    struct StubProvider;
    #[async_trait]
    impl EmbeddingProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }
        fn model_id(&self) -> &str {
            "stub-model"
        }
        fn dimensions(&self) -> usize {
            4
        }
        async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32, 1.0, 0.0, 0.0])
                .collect())
        }
    }

    let retriever = build_retriever(Arc::new(StubProvider));
    retriever
        .index(vec![(
            "d1".into(),
            "cats".into(),
            json!({"path_scope": "s"}),
        )])
        .await
        .expect("index");
    let hits = retriever.retrieve("cats", 1).await.expect("retrieve");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}
