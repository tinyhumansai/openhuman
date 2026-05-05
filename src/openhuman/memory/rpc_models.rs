//! RPC data models for the OpenHuman memory system.
//!
//! This module defines the request and response structures used by the JSON-RPC
//! interface to interact with the memory system. These models ensure type-safe
//! communication between the frontend/client and the Rust backend.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Standard error structure for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    /// A machine-readable error code.
    pub code: String,
    /// A human-readable error message.
    pub message: String,
    /// Optional additional error details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Pagination metadata for list-based responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    /// Maximum number of items requested.
    pub limit: usize,
    /// Number of items skipped.
    pub offset: usize,
    /// Total number of items available in the backend.
    pub count: usize,
}

/// General metadata included in all API envelopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMeta {
    /// Unique identifier for the request.
    pub request_id: String,
    /// Time taken to process the request in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_seconds: Option<f64>,
    /// Whether the response was served from a cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// Optional counts of various items (e.g., by category).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<BTreeMap<String, usize>>,
    /// Optional pagination information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

/// Generic envelope for all API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEnvelope<T> {
    /// The actual payload of the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Error information if the request failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    /// Metadata about the request and response.
    pub meta: ApiMeta,
}

/// An empty request body for methods that don't require parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyRequest {}

/// Request to create a new conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateConversationThreadRequest {
    #[serde(default)]
    pub labels: Option<Vec<String>>,
}

/// Request payload for `openhuman.memory_init`.
///
/// `jwt_token` is accepted for backward compatibility but **not used** — memory
/// is local-only (SQLite). Remote/cloud memory sync is a future consideration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryInitRequest {
    /// Optional token, currently ignored as memory is local-only.
    #[serde(default)]
    pub jwt_token: Option<String>,
}

/// Response payload for `openhuman.memory_init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInitResponse {
    /// Whether the memory system was successfully initialized.
    pub initialized: bool,
    /// The root workspace directory.
    pub workspace_dir: String,
    /// The specific directory where memory data is stored.
    pub memory_dir: String,
}

/// Summary information for a workspace-backed conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThreadSummary {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<i64>,
    pub is_active: bool,
    pub message_count: usize,
    pub last_message_at: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// A single persisted conversation message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessageRecord {
    pub id: String,
    pub content: String,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub extra_metadata: serde_json::Value,
    pub sender: String,
    pub created_at: String,
}

/// Request to create or update a thread in workspace storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertConversationThreadRequest {
    pub id: String,
    pub title: String,
    pub created_at: String,
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
}

/// Request to update labels for a conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConversationThreadLabelsRequest {
    pub thread_id: String,
    pub labels: Vec<String>,
}

/// Response payload for thread list operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThreadsListResponse {
    pub threads: Vec<ConversationThreadSummary>,
    pub count: usize,
}

/// Request to fetch messages for a specific thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationMessagesRequest {
    pub thread_id: String,
}

/// Response payload for message list operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessagesResponse {
    pub messages: Vec<ConversationMessageRecord>,
    pub count: usize,
}

/// Request to append a message to a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendConversationMessageRequest {
    pub thread_id: String,
    pub message: ConversationMessageRecord,
}

/// Request to generate or refresh a thread title after the first exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateConversationThreadTitleRequest {
    pub thread_id: String,
    #[serde(default)]
    pub assistant_message: Option<String>,
}

/// Request to patch a persisted message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConversationMessageRequest {
    pub thread_id: String,
    pub message_id: String,
    #[serde(default)]
    pub extra_metadata: Option<serde_json::Value>,
}

/// Request to delete a thread and its message log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteConversationThreadRequest {
    pub thread_id: String,
    pub deleted_at: String,
}

/// Response payload for single-thread deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteConversationThreadResponse {
    pub deleted: bool,
}

/// Response payload for purging all workspace-backed conversations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeConversationThreadsResponse {
    pub messages_deleted: usize,
    pub agent_threads_deleted: usize,
    pub agent_messages_deleted: usize,
}

/// Request payload for `openhuman.list_documents`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDocumentsRequest {
    /// Optional namespace filter.
    #[serde(default)]
    pub namespace: Option<String>,
}

/// Summary information for a document in memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDocumentSummary {
    /// Unique identifier for the document.
    pub document_id: String,
    /// Namespace the document belongs to.
    pub namespace: String,
    /// Lookup key for the document.
    pub key: String,
    /// Human-readable title.
    pub title: String,
    /// Type of the source (e.g., "file", "web", "note").
    pub source_type: String,
    /// Ingestion priority.
    pub priority: String,
    /// Creation timestamp (Unix epoch).
    pub created_at: f64,
    /// Last update timestamp (Unix epoch).
    pub updated_at: f64,
}

/// Response payload for `openhuman.list_documents`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDocumentsResponse {
    /// The namespace used for filtering.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The list of document summaries.
    pub documents: Vec<MemoryDocumentSummary>,
    /// Total number of documents found.
    pub count: usize,
}

/// Response payload for `openhuman.list_namespaces`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListNamespacesResponse {
    /// List of available namespace names.
    pub namespaces: Vec<String>,
    /// Total number of namespaces.
    pub count: usize,
}

/// Request payload for `openhuman.delete_document`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteDocumentRequest {
    /// Namespace containing the document.
    pub namespace: String,
    /// ID of the document to delete.
    pub document_id: String,
}

/// Response payload for `openhuman.delete_document`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteDocumentResponse {
    /// Status message of the operation.
    pub status: String,
    /// Namespace of the document.
    pub namespace: String,
    /// ID of the deleted document.
    pub document_id: String,
    /// Whether the deletion was successful.
    pub deleted: bool,
}

/// Request payload for `openhuman.query_namespace`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryNamespaceRequest {
    /// Namespace to query.
    pub namespace: String,
    /// Natural language query or search term.
    pub query: String,
    /// Whether to include reference citations in the response.
    #[serde(default)]
    pub include_references: Option<bool>,
    /// Optional filter to specific document IDs.
    #[serde(default)]
    pub document_ids: Option<Vec<String>>,
    /// Maximum number of results to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Alias for limit, specifying max number of chunks.
    #[serde(default)]
    pub max_chunks: Option<u32>,
}

impl QueryNamespaceRequest {
    /// Resolves the effective limit from `max_chunks`, `limit`, or a default value.
    pub fn resolved_limit(&self) -> u32 {
        self.max_chunks.or(self.limit).unwrap_or(10)
    }
}

/// Response payload for `openhuman.query_namespace`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryNamespaceResponse {
    /// Retrieved context including entities, relations, and chunks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<MemoryRetrievalContext>,
    /// A formatted message suitable for inclusion in an LLM prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_context_message: Option<String>,
}

/// Request payload for `openhuman.recall_context`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallContextRequest {
    /// Namespace to recall from.
    pub namespace: String,
    /// Whether to include references.
    #[serde(default)]
    pub include_references: Option<bool>,
    /// Maximum number of results.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Maximum number of chunks.
    #[serde(default)]
    pub max_chunks: Option<u32>,
}

impl RecallContextRequest {
    /// Resolves the effective limit.
    pub fn resolved_limit(&self) -> u32 {
        self.max_chunks.or(self.limit).unwrap_or(10)
    }
}

/// Response payload for `openhuman.recall_context`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallContextResponse {
    /// Retrieved context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<MemoryRetrievalContext>,
    /// Formatted LLM message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_context_message: Option<String>,
}

/// Request payload for `openhuman.recall_memories`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallMemoriesRequest {
    /// Namespace to recall from.
    pub namespace: String,
    /// Minimum retention score (0.0 to 1.0).
    #[serde(default)]
    pub min_retention: Option<f32>,
    /// Temporal filter (Unix epoch).
    #[serde(default)]
    pub as_of: Option<f64>,
    /// Maximum results.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Alias for limit.
    #[serde(default)]
    pub max_chunks: Option<u32>,
    /// Alias for limit (top K results).
    #[serde(default)]
    pub top_k: Option<u32>,
}

impl RecallMemoriesRequest {
    /// Resolves the effective limit checking `top_k`, `max_chunks`, and `limit`.
    pub fn resolved_limit(&self) -> u32 {
        self.top_k.or(self.max_chunks).or(self.limit).unwrap_or(10)
    }
}

/// Represents an entity retrieved from memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalEntity {
    /// Unique identifier for the entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Name of the entity.
    pub name: String,
    /// Type of the entity (e.g., "Person", "Place").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Retrieval relevance score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Additional arbitrary metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Represents a relationship between two entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalRelation {
    /// The subject entity.
    pub subject: String,
    /// The relationship type (predicate).
    pub predicate: String,
    /// The object entity.
    pub object: String,
    /// Relevance score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Number of times this relation was evidenced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_count: Option<u32>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Represents a text chunk retrieved from memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalChunk {
    /// ID of the chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    /// ID of the parent document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    /// The text content of the chunk.
    pub content: String,
    /// Relevance score.
    pub score: f64,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Last update timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Container for all retrieved memory components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalContext {
    /// List of entities found.
    pub entities: Vec<MemoryRetrievalEntity>,
    /// List of relations between entities.
    pub relations: Vec<MemoryRetrievalRelation>,
    /// List of raw text chunks.
    pub chunks: Vec<MemoryRetrievalChunk>,
}

/// A specific item recalled from memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallItem {
    /// Type of memory item (e.g., "fact", "observation").
    #[serde(rename = "type")]
    pub kind: String,
    /// Unique ID of the item.
    pub id: String,
    /// Text content of the memory.
    pub content: String,
    /// Relevance score.
    pub score: f64,
    /// Retention strength (0.0 to 1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<f64>,
    /// Timestamp of last access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<String>,
    /// Total number of times this memory was accessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_count: Option<u32>,
    /// How many days the memory has remained stable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability_days: Option<f64>,
}

/// Response payload for `openhuman.recall_memories`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallMemoriesResponse {
    /// List of recalled memory items.
    pub memories: Vec<MemoryRecallItem>,
}

/// Request payload for `openhuman.list_memory_files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListMemoryFilesRequest {
    /// Directory path relative to the memory root.
    #[serde(default = "default_memory_relative_dir")]
    pub relative_dir: String,
}

/// Response payload for `openhuman.list_memory_files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMemoryFilesResponse {
    /// The directory listed.
    pub relative_dir: String,
    /// List of filenames.
    pub files: Vec<String>,
    /// Total count of files.
    pub count: usize,
}

/// Request payload for `openhuman.read_memory_file`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadMemoryFileRequest {
    /// Path to the file relative to the memory root.
    pub relative_path: String,
}

/// Response payload for `openhuman.read_memory_file`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadMemoryFileResponse {
    /// The path of the file read.
    pub relative_path: String,
    /// Full content of the file.
    pub content: String,
}

/// Request payload for `openhuman.write_memory_file`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteMemoryFileRequest {
    /// Path to write to relative to the memory root.
    pub relative_path: String,
    /// Content to write.
    pub content: String,
}

/// Response payload for `openhuman.write_memory_file`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteMemoryFileResponse {
    /// The path of the file written.
    pub relative_path: String,
    /// Whether the write was successful.
    pub written: bool,
    /// Number of bytes written.
    pub bytes_written: usize,
}

/// Default directory for memory operations. Empty string means the memory
/// root itself (`<workspace>/memory`); the file-based memory RPCs resolve all
/// relative paths under that directory.
fn default_memory_relative_dir() -> String {
    String::new()
}

#[cfg(test)]
#[path = "rpc_models_tests.rs"]
mod tests;
