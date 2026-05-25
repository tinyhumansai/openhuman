//! Public input/output types for namespace memory documents.

use serde::{Deserialize, Serialize};

pub(crate) const GLOBAL_NAMESPACE: &str = "global";

/// Input payload for upserting a namespace-scoped memory document.
///
/// Used by `MemoryClient::put_doc` and the ingestion pipeline. `document_id`
/// is optional — when omitted, an existing row keyed by `(namespace, key)` is
/// reused, otherwise a new id is generated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceDocumentInput {
    pub namespace: String,
    pub key: String,
    pub title: String,
    pub content: String,
    pub source_type: String,
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub category: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub document_id: Option<String>,
}

/// One ranked retrieval result for a namespace text query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceQueryResult {
    pub key: String,
    pub content: String,
    pub score: f64,
    /// Stored category string (e.g. `core`, `daily`, or custom label).
    pub category: String,
}

/// Discriminator for the kind of stored memory item a hit refers to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryItemKind {
    Document,
    Kv,
    Episodic,
    Event,
}

/// Persisted form of a memory document as stored in `memory_docs`,
/// including timestamps and the markdown sidecar path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMemoryDocument {
    pub document_id: String,
    pub namespace: String,
    pub key: String,
    pub title: String,
    pub content: String,
    pub source_type: String,
    pub priority: String,
    pub tags: Vec<String>,
    pub metadata: serde_json::Value,
    pub category: String,
    pub session_id: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub markdown_rel_path: String,
}

/// A single KV row, namespace-scoped or global (when `namespace` is `None`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryKvRecord {
    pub namespace: Option<String>,
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: f64,
}

/// A graph edge (subject — predicate → object) plus accumulated evidence.
///
/// `document_ids` and `chunk_ids` track every source that contributed to this
/// relation; `evidence_count` is the merged count after de-duplication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRelationRecord {
    pub namespace: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub attrs: serde_json::Value,
    pub updated_at: f64,
    pub evidence_count: u32,
    pub order_index: Option<i64>,
    pub document_ids: Vec<String>,
    pub chunk_ids: Vec<String>,
}

/// Per-signal contribution to a hit's final score, surfaced for debugging
/// and UI ranking explainers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RetrievalScoreBreakdown {
    pub keyword_relevance: f64,
    pub vector_similarity: f64,
    pub graph_relevance: f64,
    pub episodic_relevance: f64,
    pub freshness: f64,
    pub final_score: f64,
}

/// A single ranked retrieval hit returned from `query_namespace_hits` /
/// `recall_namespace_memories`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceMemoryHit {
    pub id: String,
    pub kind: MemoryItemKind,
    pub namespace: String,
    pub key: String,
    pub title: Option<String>,
    pub content: String,
    pub category: String,
    pub source_type: Option<String>,
    pub updated_at: f64,
    pub score: f64,
    pub score_breakdown: RetrievalScoreBreakdown,
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(default)]
    pub chunk_id: Option<String>,
    #[serde(default)]
    pub supporting_relations: Vec<GraphRelationRecord>,
}

/// Aggregated retrieval result for a namespace: rendered context text plus
/// the underlying hits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceRetrievalContext {
    pub namespace: String,
    pub query: Option<String>,
    pub context_text: String,
    pub hits: Vec<NamespaceMemoryHit>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn global_namespace_constant_is_stable() {
        assert_eq!(GLOBAL_NAMESPACE, "global");
    }

    #[test]
    fn memory_item_kind_serde_uses_snake_case() {
        let json_value = serde_json::to_string(&MemoryItemKind::Document).unwrap();
        assert_eq!(json_value, "\"document\"");
        let decoded: MemoryItemKind = serde_json::from_str("\"episodic\"").unwrap();
        assert_eq!(decoded, MemoryItemKind::Episodic);
    }

    #[test]
    fn namespace_document_input_defaults_optional_fields() {
        let value = json!({
            "namespace": "global",
            "key": "note-1",
            "title": "Title",
            "content": "Body",
            "source_type": "manual",
            "priority": "normal",
            "metadata": {},
            "category": "core"
        });
        let parsed: NamespaceDocumentInput = serde_json::from_value(value).unwrap();
        assert!(parsed.tags.is_empty());
        assert_eq!(parsed.metadata, json!({}));
        assert!(parsed.session_id.is_none());
        assert!(parsed.document_id.is_none());
    }

    #[test]
    fn retrieval_score_breakdown_default_is_zeroed() {
        let breakdown = RetrievalScoreBreakdown::default();
        assert_eq!(breakdown.keyword_relevance, 0.0);
        assert_eq!(breakdown.vector_similarity, 0.0);
        assert_eq!(breakdown.graph_relevance, 0.0);
        assert_eq!(breakdown.episodic_relevance, 0.0);
        assert_eq!(breakdown.freshness, 0.0);
        assert_eq!(breakdown.final_score, 0.0);
    }

    #[test]
    fn memory_kv_record_roundtrips_with_optional_namespace() {
        let global = MemoryKvRecord {
            namespace: None,
            key: "theme".into(),
            value: json!("dark"),
            updated_at: 1.5,
        };
        let namespaced = MemoryKvRecord {
            namespace: Some("project".into()),
            key: "state".into(),
            value: json!({"open": true}),
            updated_at: 2.5,
        };
        for record in [global, namespaced] {
            let value = serde_json::to_value(&record).unwrap();
            let decoded: MemoryKvRecord = serde_json::from_value(value).unwrap();
            assert_eq!(decoded.namespace, record.namespace);
            assert_eq!(decoded.key, record.key);
            assert_eq!(decoded.value, record.value);
            assert_eq!(decoded.updated_at, record.updated_at);
        }
    }

    #[test]
    fn namespace_memory_hit_defaults_optional_fields() {
        let hit: NamespaceMemoryHit = serde_json::from_value(json!({
            "id": "hit-1",
            "kind": "document",
            "namespace": "global",
            "key": "note-1",
            "title": "Title",
            "content": "Body",
            "category": "core",
            "source_type": "manual",
            "updated_at": 3.5,
            "score": 0.8,
            "score_breakdown": {
                "keyword_relevance": 0.5,
                "vector_similarity": 0.2,
                "graph_relevance": 0.0,
                "episodic_relevance": 0.0,
                "freshness": 0.1,
                "final_score": 0.8
            }
        }))
        .unwrap();

        assert!(hit.document_id.is_none());
        assert!(hit.chunk_id.is_none());
        assert!(hit.supporting_relations.is_empty());
        assert_eq!(hit.kind, MemoryItemKind::Document);
    }
}
