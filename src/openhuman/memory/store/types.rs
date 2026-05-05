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
