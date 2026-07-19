//! Retriever facade over OpenHuman's recall path (issue #4249, workstream
//! 09-embeddings, steps 2 + 3).
//!
//! Step 1 landed [`ProviderEmbeddingModel`](super::ProviderEmbeddingModel), the
//! adapter that re-exposes an OpenHuman `EmbeddingProvider` as the crate's
//! provider-neutral [`EmbeddingModel`](TaEmbeddingModel). This module is the
//! **retrieval seam** the harness recall path loads context through:
//! [`recall_through_facade`] wraps OpenHuman's authoritative
//! [`Memory::recall`], projects each recalled document onto the crate's
//! [`ScoredDoc`] shape, applies the CLAUDE.md `path_scope` dedupe rule, and
//! emits [`AgentEvent::MemoryLoaded`] so retrieval becomes swappable and
//! event-visible.
//!
//! ## Adapter-first, parity over purity
//!
//! The migration plan's acceptance criterion is **recall-injection parity** —
//! the injected `[Memory context]` / `[Cross-chat context]` blocks and the
//! `collect_recall_citations` output must stay byte-identical to today. So the
//! facade **wraps** [`Memory::recall`] (preserving OpenHuman's ranking / MMR /
//! diversity engine verbatim) rather than replacing the ranking engine with a
//! fresh [`Retriever`] over an [`InMemoryVectorStore`]. The concrete crate
//! [`Retriever`] is still exposed here via [`build_retriever`] as the available,
//! swap-in engine seam (exercised in tests), so a future step can flip the
//! engine without touching the callers — but the live path returns the exact
//! same `Vec<MemoryEntry>` OpenHuman's recall produced, in the same order.
//!
//! ## `path_scope` dedupe (CLAUDE.md)
//!
//! Per the project rule, *per-item IDs are dedupe keys only*; `path_scope` is
//! the **stable collection scope**. The facade carries a derived `path_scope`
//! into each [`ScoredDoc::metadata`] and collapses the projection by the
//! per-item `id` key. Because OpenHuman recall returns unique ids within one
//! result, this collapse is a no-op on real data — which is exactly what keeps
//! the returned entries (and therefore the rendered recall block) byte-identical.
//!
//! ## Deferred (09.4)
//!
//! The embedding **usage/cost** record (crate `Usage` + catalog pricing on each
//! embed call) is *not* wired here — it is workstream 09.4, coordinated with 06.
//! This slice only builds the retrieval seam + the `MemoryLoaded` emission.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use serde_json::json;
use tinyagents::harness::embeddings::{
    EmbeddingModel as TaEmbeddingModel, InMemoryVectorStore, Retriever, ScoredDoc,
};
use tinyagents::harness::events::{AgentEvent, EventSink};

use super::ProviderEmbeddingModel;
use crate::openhuman::embeddings::EmbeddingProvider;
use crate::openhuman::memory::{Memory, MemoryEntry, RecallOpts};

/// Process-global [`EventSink`] the facade emits [`AgentEvent::MemoryLoaded`]
/// onto.
///
/// OpenHuman assembles its recall/context block *before* the tinyagents
/// [`RunContext`](tinyagents::harness::context::RunContext) (and its per-run
/// `EventSink`) exist — the block is prepended to the user message that then
/// seeds the harness turn. There is therefore no run-scoped sink in scope at
/// recall time, so memory loading has historically emitted **zero**
/// `MemoryLoaded` events. This module-level sink gives the seam a real,
/// subscribable event surface using the crate's own event path (the same
/// [`EventSink`] + [`AgentEvent`] machinery the observability bridge consumes),
/// without threading a sink through every recall call site.
fn memory_event_sink() -> &'static EventSink {
    static SINK: OnceLock<EventSink> = OnceLock::new();
    SINK.get_or_init(EventSink::new)
}

/// Derive the **stable collection scope** (`path_scope`) for a recalled entry.
///
/// Per CLAUDE.md, `path_scope` names the collection a source belongs to (the
/// dedupe *scope*), while the per-item `id` is the dedupe *key*. A recalled
/// [`MemoryEntry`] carries no explicit `path_scope` column, so we derive the
/// most stable scope available: the entry's `namespace`, falling back to the
/// `id`'s collection prefix (e.g. `episodic-cross:…` → `episodic-cross`), and
/// finally `"global"`.
fn derive_path_scope(entry: &MemoryEntry) -> String {
    if let Some(ns) = entry.namespace.as_deref() {
        if !ns.is_empty() {
            return ns.to_string();
        }
    }
    if let Some((prefix, _)) = entry.id.split_once(':') {
        if !prefix.is_empty() {
            return prefix.to_string();
        }
    }
    "global".to_string()
}

/// Project an OpenHuman [`MemoryEntry`] onto the crate [`ScoredDoc`] shape,
/// carrying the derived `path_scope` (and the identity fields the seam needs)
/// into `metadata`.
///
/// `score` maps the entry's optional relevance score (absent → `0.0`); the
/// cosine-similarity range contract is preserved because OpenHuman scores are
/// already normalised relevance values.
fn entry_to_scored_doc(entry: &MemoryEntry) -> ScoredDoc {
    ScoredDoc {
        id: entry.id.clone(),
        score: entry.score.unwrap_or(0.0) as f32,
        metadata: json!({
            "path_scope": derive_path_scope(entry),
            "key": entry.key,
            "namespace": entry.namespace,
            "session_id": entry.session_id,
            "category": entry.category.to_string(),
            "timestamp": entry.timestamp,
        }),
    }
}

/// Apply the CLAUDE.md `path_scope` dedupe rule to a [`ScoredDoc`] projection:
/// collapse by the per-item `id` key (the dedupe key), keeping the first
/// occurrence so the ranked order OpenHuman produced is preserved. `path_scope`
/// rides along in each doc's `metadata` as the collection scope.
fn dedupe_scored_docs(docs: Vec<ScoredDoc>) -> Vec<ScoredDoc> {
    let mut seen: HashSet<String> = HashSet::new();
    docs.into_iter()
        .filter(|d| seen.insert(d.id.clone()))
        .collect()
}

/// Collapse recalled entries by the same per-item `id` dedupe key the
/// [`ScoredDoc`] projection uses, preserving first-occurrence (ranked) order.
///
/// OpenHuman recall returns unique ids within a single result, so this is a
/// no-op on real data — which is the property that keeps the facade's returned
/// entries (and the rendered recall block) byte-identical to a direct
/// `Memory::recall` call.
fn dedupe_entries_by_id(entries: Vec<MemoryEntry>) -> Vec<MemoryEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    entries
        .into_iter()
        .filter(|e| seen.insert(e.id.clone()))
        .collect()
}

/// Load recall context through the retrieval facade.
///
/// Wraps OpenHuman's authoritative [`Memory::recall`] (ranking engine unchanged)
/// and, additively:
/// 1. projects each recalled document onto a crate [`ScoredDoc`] carrying its
///    derived `path_scope` (the swappable-engine seam);
/// 2. applies the `path_scope` dedupe rule (per-item `id` is the dedupe key);
/// 3. emits [`AgentEvent::MemoryLoaded`] when any context was loaded;
/// 4. logs a grep-friendly `[memory]` diagnostic.
///
/// Returns the same `Vec<MemoryEntry>` `Memory::recall` produced (deduped by the
/// unique-id key = no-op on real data), so every caller's rendered recall block
/// and citation output stay byte-identical.
pub(crate) async fn recall_through_facade<'a>(
    mem: &dyn Memory,
    query: &str,
    limit: usize,
    opts: RecallOpts<'a>,
) -> anyhow::Result<Vec<MemoryEntry>> {
    // Snapshot the query-shape fields before `opts` is moved into `recall`.
    let cross_session = opts.cross_session;
    let namespace = opts.namespace.map(str::to_string);

    let entries = mem.recall(query, limit, opts).await?;

    // Project onto the crate ScoredDoc seam and apply the path_scope dedupe rule.
    let scored = dedupe_scored_docs(entries.iter().map(entry_to_scored_doc).collect());
    // Collapse the returned entries by the same id key so the facade output
    // matches the deduped projection. Unique ids => no-op (parity preserved).
    let entries = dedupe_entries_by_id(entries);

    if !entries.is_empty() {
        // The seam is now event-visible: memory loading previously emitted zero
        // `MemoryLoaded` events in OpenHuman.
        memory_event_sink().emit(AgentEvent::MemoryLoaded);
    }

    tracing::debug!(
        query_chars = query.chars().count(),
        limit,
        cross_session,
        namespace = namespace.as_deref().unwrap_or("<default>"),
        entries = entries.len(),
        scored_docs = scored.len(),
        "[memory] recall routed through tinyagents retriever facade"
    );

    Ok(entries)
}

/// Build the concrete crate [`Retriever`] over the [`ProviderEmbeddingModel`]
/// adapter and an [`InMemoryVectorStore`] — the swap-in retrieval **engine**
/// seam.
///
/// The live recall path deliberately does **not** run through this (it wraps
/// [`Memory::recall`] to preserve OpenHuman's ranking engine, per the parity
/// acceptance criterion). This constructor exists so the engine is swappable
/// without touching callers, and is exercised in tests to keep the seam live.
#[allow(dead_code)] // Engine-swap seam; the live path wraps Memory::recall for parity (09.2).
pub(crate) fn build_retriever(provider: Arc<dyn EmbeddingProvider>) -> Retriever {
    let model: Arc<dyn TaEmbeddingModel> = Arc::new(ProviderEmbeddingModel::new(provider));
    Retriever::new(model, Arc::new(InMemoryVectorStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tinyagents::harness::events::{EventListener, EventRecord};

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
}
