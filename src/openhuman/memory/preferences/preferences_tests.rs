//! Tests for the two-lane preference helpers.
//!
//! The Lane-A helper runs against the real [`InMemoryProvider`] through a real
//! guard, so it exercises the same `list` → `get` pair production uses. The
//! vector-filtered helpers need a driver that advertises
//! [`Capability::Retrieval`], which the in-memory provider deliberately does
//! not, so those use a purpose-built stub — the point under test is the
//! *filter*, not the retrieval.

use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::openhuman::memory::api::capabilities::Capabilities;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::retrieval::MemoryRetrieval;
use crate::openhuman::memory::api::provider::{
    MemoryCore, MemoryPortability, MemoryProvider, MemoryRecall,
};
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryItemKind, MemoryTaint, NamespaceMemoryHit, RetrievalScoreBreakdown,
};
use crate::openhuman::memory::guard::in_memory::{guarded_in_memory, InMemoryProvider};

#[tokio::test]
async fn load_general_preferences_returns_bodies_not_topic_keys_and_honours_the_limit() {
    let (_provider, guard) = guarded_in_memory();

    for (key, value) in [
        ("reply_language", "Reply in British English."),
        ("tone", "Be terse."),
    ] {
        guard
            .store(
                USER_PREF_GENERAL_NAMESPACE,
                key,
                value,
                MemoryCategory::Core,
                None,
                MemoryTaint::Internal,
            )
            .await
            .unwrap();
    }

    let general = load_general_preferences(&guard, 10).await;
    assert!(general.iter().any(|v| v.contains("British English")));
    assert!(general.iter().any(|v| v.contains("Be terse")));
    // The bodies, never the topic keys — the bug this helper exists to avoid.
    assert!(!general.iter().any(|v| v == "reply_language"));

    assert_eq!(load_general_preferences(&guard, 1).await.len(), 1);
}

/// A blank entry must not consume the caller's budget.
///
/// `list()` returns newest-first, so a blank newest entry sat in front of the
/// real ones. Truncating to `limit` before dropping blanks meant a caller
/// asking for one preference got none — the Lane-A prompt block silently lost
/// a standing preference, and nothing anywhere reported a problem.
#[tokio::test]
async fn a_blank_newest_entry_does_not_consume_the_limit() {
    let (_provider, guard) = guarded_in_memory();

    // Stored oldest-first so the blank one is newest and is seen first.
    for (key, value) in [("tone", "Be terse."), ("scratch", "   ")] {
        guard
            .store(
                USER_PREF_GENERAL_NAMESPACE,
                key,
                value,
                MemoryCategory::Core,
                None,
                MemoryTaint::Internal,
            )
            .await
            .unwrap();
    }

    let general = load_general_preferences(&guard, 1).await;
    assert_eq!(
        general,
        vec!["Be terse.".to_string()],
        "a blank newest entry must be skipped, not counted against the limit"
    );
}

/// A driver whose only real family is retrieval, answering with hits whose
/// vector component is set per-entry so the filter can be observed.
struct ScriptedRetrieval {
    hits: Vec<(String, String, f64)>,
}

#[async_trait]
impl MemoryRetrieval for ScriptedRetrieval {
    async fn recall_namespace_scored(
        &self,
        namespace: &str,
        _query: &str,
        limit: usize,
        _exclude_session_id: Option<&str>,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        Ok(self
            .hits
            .iter()
            .take(limit)
            .map(|(key, content, vector)| NamespaceMemoryHit {
                id: key.clone(),
                kind: MemoryItemKind::Document,
                namespace: namespace.to_string(),
                key: key.clone(),
                title: None,
                content: content.clone(),
                category: "core".to_string(),
                source_type: None,
                updated_at: 0.0,
                // Deliberately high, and independent of the vector component:
                // a filter that read this instead would pass everything.
                score: 1.0,
                score_breakdown: RetrievalScoreBreakdown {
                    keyword_relevance: 0.0,
                    vector_similarity: *vector,
                    graph_relevance: 0.0,
                    episodic_relevance: 0.0,
                    freshness: 0.0,
                    final_score: 1.0,
                },
                document_id: None,
                chunk_id: None,
                supporting_relations: Vec::new(),
                taint: MemoryTaint::Internal,
            })
            .collect())
    }

    async fn recall_namespace_recent(
        &self,
        namespace: &str,
        limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        // The scripted hits, minus the ranking the scored path pretends to do.
        self.recall_namespace_scored(namespace, "", limit, None)
            .await
    }

    // The family's other methods are irrelevant here — this stub exists to feed
    // `recall_namespace_scored` a scripted breakdown. They are unreachable, so
    // they say so rather than returning a plausible empty value that could make
    // a future test pass for the wrong reason.
    async fn fast_retrieve(
        &self,
        _query: &str,
        _options: crate::openhuman::memory::api::provider::retrieval::FastRetrieveQuery,
        _scope: Option<&crate::openhuman::memory::api::provider::types::SourceScope>,
    ) -> Result<crate::openhuman::memory::api::provider::retrieval::RetrievalResponse, MemoryError>
    {
        unimplemented!("ScriptedRetrieval only serves recall_namespace_scored")
    }

    async fn cover_window(
        &self,
        _window: &crate::openhuman::memory::api::provider::retrieval::CoverWindowQuery,
        _scope: Option<&crate::openhuman::memory::api::provider::types::SourceScope>,
    ) -> Result<crate::openhuman::memory::api::provider::retrieval::RetrievalResponse, MemoryError>
    {
        unimplemented!("ScriptedRetrieval only serves recall_namespace_scored")
    }

    async fn retrieve_source(
        &self,
        _query: &crate::openhuman::memory::api::provider::retrieval::SourceRetrievalQuery,
        _scope: Option<&crate::openhuman::memory::api::provider::types::SourceScope>,
    ) -> Result<crate::openhuman::memory::api::provider::retrieval::RetrievalResponse, MemoryError>
    {
        unimplemented!("ScriptedRetrieval only serves recall_namespace_scored")
    }

    async fn retrieve_children(
        &self,
        _node_id: &str,
        _max_depth: u32,
        _query: Option<&str>,
        _limit: Option<usize>,
        _scope: Option<&crate::openhuman::memory::api::provider::SourceScope>,
    ) -> Result<Vec<crate::openhuman::memory::api::provider::retrieval::RetrievalHit>, MemoryError>
    {
        unimplemented!("ScriptedRetrieval only serves recall_namespace_scored")
    }

    async fn retrieve_leaves(
        &self,
        _chunk_ids: &[String],
        _scope: Option<&crate::openhuman::memory::api::provider::SourceScope>,
    ) -> Result<Vec<crate::openhuman::memory::api::provider::retrieval::RetrievalHit>, MemoryError>
    {
        unimplemented!("ScriptedRetrieval only serves recall_namespace_scored")
    }

    async fn search_entities(
        &self,
        _query: &str,
        _kinds: Option<&[String]>,
        _limit: usize,
    ) -> Result<Vec<crate::openhuman::memory::api::provider::retrieval::EntityMatch>, MemoryError>
    {
        unimplemented!("ScriptedRetrieval only serves recall_namespace_scored")
    }
}

/// Wraps [`InMemoryProvider`] so the mandatory three are real, and adds
/// retrieval on top.
struct RetrievalProvider {
    base: InMemoryProvider,
    retrieval: ScriptedRetrieval,
}

// Delegates the mandatory families to the in-memory base. Written out rather
// than reached for via a macro: it is three small traits, and the explicit form
// makes it obvious that only `as_retrieval` below is new behaviour.
#[async_trait]
impl MemoryCore for RetrievalProvider {
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        self.base
            .store(namespace, key, content, category, session_id, taint)
            .await
    }

    async fn get(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<crate::openhuman::memory::api::types::MemoryEntry>, MemoryError> {
        self.base.get(namespace, key).await
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        self.base.forget(namespace, key).await
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::openhuman::memory::api::types::MemoryEntry>, MemoryError> {
        self.base.list(namespace, category, session_id).await
    }

    async fn namespaces(
        &self,
    ) -> Result<Vec<crate::openhuman::memory::api::types::NamespaceSummary>, MemoryError> {
        self.base.namespaces().await
    }
}

#[async_trait]
impl MemoryRecall for RetrievalProvider {
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &crate::openhuman::memory::api::recall::OwnedRecallOpts,
        scope: Option<&crate::openhuman::memory::api::provider::types::SourceScope>,
    ) -> Result<Vec<crate::openhuman::memory::api::types::MemoryEntry>, MemoryError> {
        self.base.recall(query, limit, opts, scope).await
    }
}

#[async_trait]
impl MemoryPortability for RetrievalProvider {
    async fn export_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<crate::openhuman::memory::api::provider::types::ExportPage, MemoryError> {
        self.base.export_page(cursor, limit).await
    }

    async fn import_records(
        &self,
        records: Vec<crate::openhuman::memory::api::provider::types::ExportRecord>,
    ) -> Result<crate::openhuman::memory::api::provider::types::ImportOutcome, MemoryError> {
        self.base.import_records(records).await
    }
}

#[async_trait]
impl MemoryProvider for RetrievalProvider {
    fn driver_id(&self) -> &str {
        "scripted-retrieval"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::mandatory()
            .with(crate::openhuman::memory::api::capabilities::Capability::Retrieval)
    }

    fn as_retrieval(&self) -> Option<&dyn MemoryRetrieval> {
        Some(&self.retrieval)
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
    }
}

fn scripted(hits: Vec<(&str, &str, f64)>) -> Arc<MemoryGuard> {
    let provider = Arc::new(RetrievalProvider {
        base: InMemoryProvider::new(),
        retrieval: ScriptedRetrieval {
            hits: hits
                .into_iter()
                .map(|(k, c, v)| (k.to_string(), c.to_string(), v))
                .collect(),
        },
    });
    crate::openhuman::memory::guard::in_memory::guard_over(provider)
}

#[tokio::test]
async fn situational_recall_filters_on_the_vector_component_not_the_final_score() {
    // Every hit has final_score 1.0; only the vector component separates them.
    let guard = scripted(vec![
        ("editor", "Prefers vim.", 0.9),
        ("lexical_only", "Shares words, means nothing.", 0.1),
    ]);

    let out = recall_situational_preferences(&guard, "which editor?").await;
    assert_eq!(out, vec!["Prefers vim.".to_string()]);
}

#[tokio::test]
async fn an_empty_query_recalls_nothing_without_asking_the_driver() {
    let guard = scripted(vec![("editor", "Prefers vim.", 0.99)]);
    assert!(recall_situational_preferences(&guard, "   ")
        .await
        .is_empty());
}

#[tokio::test]
async fn a_driver_without_retrieval_yields_no_preferences_rather_than_an_error() {
    let (_provider, guard) = guarded_in_memory();
    assert!(recall_situational_preferences(&guard, "anything")
        .await
        .is_empty());
    assert!(recall_related_preferences(&guard, "some value", "topic", 4)
        .await
        .is_empty());
}

#[tokio::test]
async fn related_preferences_exclude_the_just_saved_topic() {
    let guard = scripted(vec![
        ("tone", "Be terse.", 0.9),
        ("verbosity", "Be brief.", 0.9),
    ]);

    let related = recall_related_preferences(&guard, "Be brief.", "verbosity", 4).await;
    let topics: Vec<&str> = related.iter().map(|(t, _)| t.as_str()).collect();
    assert!(topics.contains(&"tone"));
    assert!(
        !topics.contains(&"verbosity"),
        "the preference just written must not be surfaced as contradicting itself"
    );
}

#[tokio::test]
async fn the_limit_is_a_budget_shared_across_both_lanes() {
    // The stub answers identically for both namespaces, so an unshared budget
    // would return `limit` per lane — twice what the caller asked for.
    let guard = scripted(vec![
        ("a", "Alpha.", 0.9),
        ("b", "Bravo.", 0.9),
        ("c", "Charlie.", 0.9),
    ]);

    let related = recall_related_preferences(&guard, "anything", "none", 2).await;
    assert_eq!(related.len(), 2);
}
