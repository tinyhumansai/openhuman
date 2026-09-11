//! See the section comment below — split out of `rpc_models.rs` for the
//! file-layout gate; every name is re-exported from there.

use serde::{Deserialize, Serialize};

// ── Document-ingestion wire shapes ──────────────────────────────────────────
//
// These were `pub use tinycortex::memory::ingest::{…}` in `memory/mod.rs` —
// the last re-export shim standing between this crate and dropping the engine
// from `[dependencies]`. `doc_ingest` routes through `MemoryDocuments::
// put_document` and fills the result with host-known facts, so nothing here
// drives the engine any more; what survives is the WIRE SHAPE the dashboard
// already parses. Copied field-for-field (camelCase, defaults, required
// `modelName`) from `tinycortex::memory::ingest::extract::types`; the serde
// tests below pin the shape literal-for-literal so drift is a test failure,
// not a silent contract change.

/// The granularity of heuristic extraction. Accepted for wire compatibility;
/// extraction itself is driver-owned behind `put_document`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMode {
    /// Extract from each individual sentence (higher precision).
    #[default]
    Sentence,
    /// Extract from the entire chunk at once (faster, better for context).
    Chunk,
}

/// Ingestion tuning accepted on `memory.doc_ingest`. The driver owns
/// extraction, so the host validates the shape and passes nothing on — but a
/// payload the old engine accepted must still deserialize here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestionConfig {
    /// Extraction backend label recorded in metadata/results.
    pub model_name: String,
    /// The granularity of heuristic extraction.
    #[serde(default)]
    pub extraction_mode: ExtractionMode,
    /// Minimum confidence threshold for entity extraction (0.0 to 1.0).
    #[serde(default = "default_entity_threshold")]
    pub entity_threshold: f32,
    /// Minimum confidence threshold for relation extraction (0.0 to 1.0).
    #[serde(default = "default_relation_threshold")]
    pub relation_threshold: f32,
    /// Threshold for adjacency-based heuristics.
    #[serde(default = "default_adjacency_threshold")]
    pub adjacency_threshold: f32,
    /// Reserved batch-size knob kept for config compatibility.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_entity_threshold() -> f32 {
    0.45
}

fn default_relation_threshold() -> f32 {
    0.30
}

fn default_adjacency_threshold() -> f32 {
    0.50
}

fn default_batch_size() -> usize {
    16
}

/// An entity identified during ingestion. Always empty in module-driver
/// responses (extraction details are driver-internal); the field type stays so
/// the response keeps its `entities: []` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedEntity {
    /// Normalized name of the entity (all-caps).
    pub name: String,
    /// Classification (e.g., PERSON, ORGANIZATION).
    pub entity_type: String,
    /// Known aliases for this entity.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// A relation identified during ingestion. Same story as [`ExtractedEntity`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedRelation {
    /// Name of the subject entity.
    pub subject: String,
    /// Classification of the subject.
    pub subject_type: String,
    /// Relationship type (e.g., OWNS, WORKS_ON).
    pub predicate: String,
    /// Name of the object entity.
    pub object: String,
    /// Classification of the object.
    pub object_type: String,
    /// Extraction confidence (0.0 to 1.0).
    pub confidence: f32,
    /// Number of distinct occurrences of this relation.
    pub evidence_count: u32,
    /// IDs of the chunks where this relation was found.
    pub chunk_ids: Vec<String>,
    /// Sequential order index for reconstruction.
    pub order_index: Option<i64>,
    /// Additional metadata about the extraction.
    pub metadata: serde_json::Value,
}

/// The `memory.doc_ingest` response. Counts are zero and labels read
/// `driver-managed` under the module driver — `put_document` owns chunking and
/// extraction, and a module boundary cannot observe their details.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestionResult {
    /// ID of the document that was ingested.
    pub document_id: String,
    /// Namespace containing the document.
    pub namespace: String,
    /// Extraction backend label recorded for the ingestion run.
    pub model_name: String,
    /// Mode used for extraction.
    pub extraction_mode: String,
    /// Total number of chunks processed.
    pub chunk_count: usize,
    /// Total number of distinct entities found.
    pub entity_count: usize,
    /// Total number of distinct relations found.
    pub relation_count: usize,
    /// Number of identified user preferences.
    pub preference_count: usize,
    /// Number of identified decisions.
    pub decision_count: usize,
    /// Auto-generated tags for the document.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Complete list of identified entities.
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,
    /// Complete list of identified relations.
    #[serde(default)]
    pub relations: Vec<ExtractedRelation>,
}
