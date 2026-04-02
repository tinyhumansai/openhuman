use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    pub limit: usize,
    pub offset: usize,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMeta {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<BTreeMap<String, usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEnvelope<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    pub meta: ApiMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyRequest {}

/// Request payload for `openhuman.memory_init`.
///
/// `jwt_token` is accepted for backward compatibility but **not used** — memory
/// is local-only (SQLite).  Remote/cloud memory sync is a future consideration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryInitRequest {
    #[serde(default)]
    pub jwt_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInitResponse {
    pub initialized: bool,
    pub workspace_dir: String,
    pub memory_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDocumentsRequest {
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDocumentSummary {
    pub document_id: String,
    pub namespace: String,
    pub key: String,
    pub title: String,
    pub source_type: String,
    pub priority: String,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDocumentsResponse {
    #[serde(default)]
    pub namespace: Option<String>,
    pub documents: Vec<MemoryDocumentSummary>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListNamespacesResponse {
    pub namespaces: Vec<String>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteDocumentRequest {
    pub namespace: String,
    pub document_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteDocumentResponse {
    pub status: String,
    pub namespace: String,
    pub document_id: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryNamespaceRequest {
    pub namespace: String,
    pub query: String,
    #[serde(default)]
    pub include_references: Option<bool>,
    #[serde(default)]
    pub document_ids: Option<Vec<String>>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub max_chunks: Option<u32>,
}

impl QueryNamespaceRequest {
    pub fn resolved_limit(&self) -> u32 {
        self.max_chunks.or(self.limit).unwrap_or(10)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryNamespaceResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<MemoryRetrievalContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_context_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallContextRequest {
    pub namespace: String,
    #[serde(default)]
    pub include_references: Option<bool>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub max_chunks: Option<u32>,
}

impl RecallContextRequest {
    pub fn resolved_limit(&self) -> u32 {
        self.max_chunks.or(self.limit).unwrap_or(10)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallContextResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<MemoryRetrievalContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_context_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallMemoriesRequest {
    pub namespace: String,
    /// Accepted for forward compatibility and currently ignored until the core
    /// recall path exposes real retention-based filtering semantics.
    #[serde(default)]
    pub min_retention: Option<f32>,
    /// Accepted for forward compatibility and currently ignored until the core
    /// recall path exposes real as-of / temporal recall semantics.
    #[serde(default)]
    pub as_of: Option<f64>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub max_chunks: Option<u32>,
    #[serde(default)]
    pub top_k: Option<u32>,
}

impl RecallMemoriesRequest {
    pub fn resolved_limit(&self) -> u32 {
        self.top_k.or(self.max_chunks).or(self.limit).unwrap_or(10)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalEntity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalRelation {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_count: Option<u32>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    pub content: String,
    pub score: f64,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalContext {
    pub entities: Vec<MemoryRetrievalEntity>,
    pub relations: Vec<MemoryRetrievalRelation>,
    pub chunks: Vec<MemoryRetrievalChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub content: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability_days: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallMemoriesResponse {
    pub memories: Vec<MemoryRecallItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListMemoryFilesRequest {
    #[serde(default = "default_memory_relative_dir")]
    pub relative_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMemoryFilesResponse {
    pub relative_dir: String,
    pub files: Vec<String>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadMemoryFileRequest {
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadMemoryFileResponse {
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteMemoryFileRequest {
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteMemoryFileResponse {
    pub relative_path: String,
    pub written: bool,
    pub bytes_written: usize,
}

fn default_memory_relative_dir() -> String {
    "memory".to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RecallMemoriesRequest;

    #[test]
    fn recall_memories_request_accepts_compatibility_noop_params() {
        let request: RecallMemoriesRequest = serde_json::from_value(json!({
            "namespace": "team",
            "top_k": 7,
            "min_retention": 0.8,
            "as_of": 1700000000.0
        }))
        .expect("compatibility params should deserialize");

        assert_eq!(request.namespace, "team");
        assert_eq!(request.top_k, Some(7));
        assert_eq!(request.min_retention, Some(0.8));
        assert_eq!(request.as_of, Some(1_700_000_000.0));
    }

    #[test]
    fn recall_memories_request_limit_resolution_ignores_compatibility_noop_params() {
        let request: RecallMemoriesRequest = serde_json::from_value(json!({
            "namespace": "team",
            "limit": 3,
            "min_retention": 0.5,
            "as_of": 1700000000.0
        }))
        .expect("request should deserialize");

        assert_eq!(request.resolved_limit(), 3);
    }
}
