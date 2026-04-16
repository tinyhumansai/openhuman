//! RPC operations for the memory system.
//!
//! This module implements the handlers for memory-related RPC requests, including
//! document management, semantic queries, key-value storage, and knowledge graph
//! operations. It manages the active memory client and provides utility functions
//! for formatting and filtering memory results.

use crate::openhuman::config::Config;
use crate::openhuman::memory::store::GraphRelationRecord;
use crate::openhuman::memory::{
    ApiEnvelope, ApiError, ApiMeta, DeleteDocumentRequest, DeleteDocumentResponse, EmptyRequest,
    ListDocumentsRequest, ListDocumentsResponse, ListMemoryFilesRequest, ListMemoryFilesResponse,
    ListNamespacesResponse, MemoryClient, MemoryClientRef, MemoryDocumentSummary,
    MemoryIngestionConfig, MemoryIngestionRequest, MemoryIngestionResult, MemoryInitRequest,
    MemoryInitResponse, MemoryItemKind, MemoryRecallItem, MemoryRetrievalChunk,
    MemoryRetrievalContext, MemoryRetrievalEntity, MemoryRetrievalRelation, NamespaceDocumentInput,
    NamespaceMemoryHit, NamespaceRetrievalContext, PaginationMeta, QueryNamespaceRequest,
    QueryNamespaceResponse, ReadMemoryFileRequest, ReadMemoryFileResponse, RecallContextRequest,
    RecallContextResponse, RecallMemoriesRequest, RecallMemoriesResponse, WriteMemoryFileRequest,
    WriteMemoryFileResponse,
};
use crate::rpc::RpcOutcome;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

// The global memory client singleton lives in `super::global`.
// All access goes through `active_memory_client()` below.

/// Generates a unique request ID for memory operations.
///
/// This ID is used for tracing and logging purposes in the API response metadata.
fn memory_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Converts an iterator of memory counts into a BTreeMap.
///
/// This is a convenience helper for populating the `counts` field in the API metadata.
fn memory_counts(
    entries: impl IntoIterator<Item = (&'static str, usize)>,
) -> BTreeMap<String, usize> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// Wraps data in an RPC API envelope.
///
/// This standardises the response format for memory-related RPC methods.
fn envelope<T: Serialize>(
    data: T,
    counts: Option<BTreeMap<String, usize>>,
    pagination: Option<PaginationMeta>,
) -> RpcOutcome<ApiEnvelope<T>> {
    RpcOutcome::new(
        ApiEnvelope {
            data: Some(data),
            error: None,
            meta: ApiMeta {
                request_id: memory_request_id(),
                latency_seconds: None,
                cached: None,
                counts,
                pagination,
            },
        },
        vec![],
    )
}

/// Wraps an error in an RPC API envelope.
///
/// This provides a consistent error reporting format for the memory system.
fn error_envelope<T: Serialize>(code: &str, message: String) -> RpcOutcome<ApiEnvelope<T>> {
    RpcOutcome::new(
        ApiEnvelope {
            data: None,
            error: Some(ApiError {
                code: code.to_string(),
                message,
                details: None,
            }),
            meta: ApiMeta {
                request_id: memory_request_id(),
                latency_seconds: None,
                cached: None,
                counts: None,
                pagination: None,
            },
        },
        vec![],
    )
}

/// Formats a floating-point timestamp as an RFC3339 string.
///
/// Returns `None` if the timestamp is invalid (NaN, infinite, or negative).
fn timestamp_to_rfc3339(timestamp: f64) -> Option<String> {
    if !timestamp.is_finite() || timestamp < 0.0 {
        return None;
    }

    let secs = timestamp.trunc() as i64;
    let nanos = ((timestamp.fract().abs()) * 1_000_000_000.0).round() as u32;
    chrono::Utc
        .timestamp_opt(secs, nanos.min(999_999_999))
        .single()
        .map(|value| value.to_rfc3339())
}

/// Maps a memory item kind to a human-readable label.
fn memory_kind_label(kind: &MemoryItemKind) -> &'static str {
    match kind {
        MemoryItemKind::Document => "document",
        MemoryItemKind::Kv => "kv",
        MemoryItemKind::Episodic => "episodic",
        MemoryItemKind::Event => "event",
    }
}

/// Generates a unique string identity for a graph relation.
///
/// The identity is composed of the namespace, subject, predicate, and object.
fn relation_identity(relation: &GraphRelationRecord) -> String {
    format!(
        "{}|{}|{}|{}",
        relation.namespace.as_deref().unwrap_or("global"),
        relation.subject.as_str(),
        relation.predicate.as_str(),
        relation.object.as_str()
    )
}

/// Formats relation metadata into a JSON Value.
fn relation_metadata(relation: &GraphRelationRecord) -> Value {
    json!({
        "namespace": relation.namespace.clone(),
        "attrs": relation.attrs.clone(),
        "order_index": relation.order_index,
        "document_ids": relation.document_ids.clone(),
        "chunk_ids": relation.chunk_ids.clone(),
        "updated_at": timestamp_to_rfc3339(relation.updated_at),
    })
}

/// Formats chunk metadata into a JSON Value.
fn chunk_metadata(hit: &NamespaceMemoryHit) -> Value {
    json!({
        "kind": memory_kind_label(&hit.kind),
        "namespace": hit.namespace.clone(),
        "key": hit.key.clone(),
        "title": hit.title.clone(),
        "category": hit.category.clone(),
        "source_type": hit.source_type.clone(),
        "score_breakdown": {
            "keyword_relevance": hit.score_breakdown.keyword_relevance,
            "vector_similarity": hit.score_breakdown.vector_similarity,
            "graph_relevance": hit.score_breakdown.graph_relevance,
            "episodic_relevance": hit.score_breakdown.episodic_relevance,
            "freshness": hit.score_breakdown.freshness,
            "final_score": hit.score_breakdown.final_score,
        }
    })
}

/// Extracts an entity type for a specific role (subject/object) from relation attributes.
fn extract_entity_type(attrs: &Value, role: &str) -> Option<String> {
    attrs
        .get("entity_types")
        .and_then(|et| et.get(role))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Transforms memory hits into a retrieval context with deduplicated entities and relations.
pub(crate) fn build_retrieval_context(hits: &[NamespaceMemoryHit]) -> MemoryRetrievalContext {
    let mut entity_types: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut relations = BTreeMap::new();
    let chunks = hits
        .iter()
        .map(|hit| {
            // Extract supporting relations from each hit to populate entities and relations
            for relation in &hit.supporting_relations {
                if !relation.subject.trim().is_empty() {
                    let entry = entity_types.entry(relation.subject.clone()).or_insert(None);
                    // Use the first non-empty entity type found for this subject
                    if entry.is_none() {
                        *entry = extract_entity_type(&relation.attrs, "subject");
                    }
                }
                if !relation.object.trim().is_empty() {
                    let entry = entity_types.entry(relation.object.clone()).or_insert(None);
                    // Use the first non-empty entity type found for this object
                    if entry.is_none() {
                        *entry = extract_entity_type(&relation.attrs, "object");
                    }
                }
                // Deduplicate relations based on their unique identity
                relations
                    .entry(relation_identity(relation))
                    .or_insert_with(|| MemoryRetrievalRelation {
                        subject: relation.subject.clone(),
                        predicate: relation.predicate.clone(),
                        object: relation.object.clone(),
                        score: None,
                        evidence_count: Some(relation.evidence_count),
                        metadata: relation_metadata(relation),
                    });
            }

            MemoryRetrievalChunk {
                chunk_id: hit.chunk_id.clone(),
                document_id: hit.document_id.clone(),
                content: hit.content.clone(),
                score: hit.score,
                metadata: chunk_metadata(hit),
                created_at: None,
                updated_at: timestamp_to_rfc3339(hit.updated_at),
            }
        })
        .collect();

    MemoryRetrievalContext {
        entities: entity_types
            .into_iter()
            .map(|(name, entity_type)| MemoryRetrievalEntity {
                id: None,
                name,
                entity_type,
                score: None,
                metadata: json!({}),
            })
            .collect(),
        relations: relations.into_values().collect(),
        chunks,
    }
}

/// Formats memory hits into a natural-language context message for LLM consumption.
fn format_llm_context_message(query: Option<&str>, hits: &[NamespaceMemoryHit]) -> Option<String> {
    if hits.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    if let Some(query) = query {
        parts.push(format!("Query: {query}"));
    }

    for hit in hits {
        let summary = match hit.kind {
            MemoryItemKind::Document => {
                let title = hit.title.clone().unwrap_or_else(|| hit.key.clone());
                format!("{title}: {}", hit.content.trim())
            }
            MemoryItemKind::Kv => format!("[kv:{}] {}", hit.key, hit.content.trim()),
            MemoryItemKind::Episodic => {
                format!("[episodic:{}] {}", hit.key, hit.content.trim())
            }
            MemoryItemKind::Event => {
                format!("[event:{}] {}", hit.key, hit.content.trim())
            }
        };
        parts.push(summary);

        // Include typed relations if present for better LLM reasoning
        if !hit.supporting_relations.is_empty() {
            let relations = hit
                .supporting_relations
                .iter()
                .map(|relation| {
                    let subject_type = extract_entity_type(&relation.attrs, "subject");
                    let object_type = extract_entity_type(&relation.attrs, "object");
                    let subject_label = match subject_type {
                        Some(t) => format!("{} ({})", relation.subject, t),
                        None => relation.subject.clone(),
                    };
                    let object_label = match object_type {
                        Some(t) => format!("{} ({})", relation.object, t),
                        None => relation.object.clone(),
                    };
                    format!(
                        "{} -[{}]-> {}",
                        subject_label, relation.predicate, object_label
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            parts.push(format!("Relations: {relations}"));
        }
    }

    Some(parts.join("\n\n"))
}

/// Filters memory hits to only include those matching specific document IDs.
fn filter_hits_by_document_ids(
    hits: Vec<NamespaceMemoryHit>,
    document_ids: Option<&[String]>,
) -> Vec<NamespaceMemoryHit> {
    let Some(document_ids) = document_ids else {
        return hits;
    };
    let allowed = document_ids.iter().cloned().collect::<BTreeSet<_>>();
    hits.into_iter()
        .filter(|hit| {
            hit.document_id
                .as_ref()
                .map(|document_id| allowed.contains(document_id))
                .unwrap_or(false)
        })
        .collect()
}

/// Returns the retrieval context if `include_references` is true and context is not empty.
fn maybe_retrieval_context(
    include_references: bool,
    context: MemoryRetrievalContext,
) -> Option<MemoryRetrievalContext> {
    if !include_references {
        return None;
    }
    if context.entities.is_empty() && context.relations.is_empty() && context.chunks.is_empty() {
        return None;
    }
    Some(context)
}

/// Returns the current workspace directory from configuration.
async fn current_workspace_dir() -> Result<PathBuf, String> {
    Config::load_or_init()
        .await
        .map(|config| config.workspace_dir)
        .map_err(|e| format!("load config: {e}"))
}

/// Returns the active memory client from the process-global singleton,
/// lazily initializing it if necessary.
async fn active_memory_client() -> Result<MemoryClientRef, String> {
    if let Some(client) = super::global::client_if_ready() {
        return Ok(client);
    }
    let workspace_dir = current_workspace_dir().await?;
    super::global::init(workspace_dir)
}

/// Validates that a relative path does not escape the memory directory.
fn validate_memory_relative_path(path: &str) -> Result<(), String> {
    let candidate = Path::new(path);
    if candidate.as_os_str().is_empty() {
        return Err("memory path must not be empty".to_string());
    }
    if candidate.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    // Prevent traversal using .. components
    for component in candidate.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path traversal is not allowed".to_string());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolves the canonical path to the memory directory within the workspace.
async fn resolve_memory_root() -> Result<PathBuf, String> {
    let workspace_dir = current_workspace_dir().await?;
    let memory_root = workspace_dir.join("memory");
    std::fs::create_dir_all(&memory_root)
        .map_err(|e| format!("create memory dir {}: {e}", memory_root.display()))?;
    memory_root
        .canonicalize()
        .map_err(|e| format!("resolve memory dir {}: {e}", memory_root.display()))
}

/// Resolves and canonicalizes an existing memory path, ensuring it stays within the workspace.
async fn resolve_existing_memory_path(relative_path: &str) -> Result<PathBuf, String> {
    validate_memory_relative_path(relative_path)?;
    let workspace_dir = current_workspace_dir().await?;
    let canonical_workspace = workspace_dir
        .canonicalize()
        .map_err(|e| format!("resolve workspace dir {}: {e}", workspace_dir.display()))?;
    let full_path = workspace_dir.join(relative_path);
    let resolved = full_path
        .canonicalize()
        .map_err(|e| format!("resolve memory path {}: {e}", full_path.display()))?;
    if !resolved.starts_with(&canonical_workspace) {
        return Err("memory path escapes the workspace directory".to_string());
    }
    Ok(resolved)
}

/// Resolves a path for writing, creating parent directories and ensuring it stays within the workspace.
async fn resolve_writable_memory_path(relative_path: &str) -> Result<PathBuf, String> {
    validate_memory_relative_path(relative_path)?;
    let workspace_dir = current_workspace_dir().await?;
    let canonical_workspace = workspace_dir
        .canonicalize()
        .map_err(|e| format!("resolve workspace dir {}: {e}", workspace_dir.display()))?;
    let full_path = workspace_dir.join(relative_path);
    let parent = full_path
        .parent()
        .ok_or_else(|| "memory path must include a file name".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("create memory path {}: {e}", parent.display()))?;
    let resolved_parent = parent
        .canonicalize()
        .map_err(|e| format!("resolve memory parent {}: {e}", parent.display()))?;
    if !resolved_parent.starts_with(&canonical_workspace) {
        return Err("memory path escapes the workspace directory".to_string());
    }
    let file_name = full_path
        .file_name()
        .ok_or_else(|| "memory path must include a file name".to_string())?;
    let resolved = resolved_parent.join(file_name);
    // Security check: refuse to write through symlinks to prevent hijacking
    if let Ok(metadata) = std::fs::symlink_metadata(&resolved) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "refusing to write through symlink: {}",
                resolved.display()
            ));
        }
    }
    Ok(resolved)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMemoryDocumentSummary {
    document_id: String,
    namespace: String,
    key: String,
    title: String,
    source_type: String,
    priority: String,
    created_at: f64,
    updated_at: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDeleteDocumentResult {
    deleted: bool,
    namespace: String,
    document_id: String,
}

fn parse_memory_document_summaries(raw: Value) -> Result<Vec<MemoryDocumentSummary>, String> {
    let documents = raw
        .get("documents")
        .and_then(Value::as_array)
        .ok_or_else(|| "memory document list missing 'documents' array".to_string())?;
    documents
        .iter()
        .cloned()
        .map(|value| {
            let raw: RawMemoryDocumentSummary = serde_json::from_value(value)
                .map_err(|e| format!("decode memory document: {e}"))?;
            Ok(MemoryDocumentSummary {
                document_id: raw.document_id,
                namespace: raw.namespace,
                key: raw.key,
                title: raw.title,
                source_type: raw.source_type,
                priority: raw.priority,
                created_at: raw.created_at,
                updated_at: raw.updated_at,
            })
        })
        .collect()
}

async fn query_limit_for_request(
    client: &MemoryClient,
    request: &QueryNamespaceRequest,
) -> Result<u32, String> {
    let requested = request.resolved_limit();
    if request.document_ids.is_none() {
        return Ok(requested);
    }

    let raw = client.list_documents(Some(&request.namespace)).await?;
    let documents = parse_memory_document_summaries(raw)?;
    let total_documents = u32::try_from(documents.len()).unwrap_or(u32::MAX);
    Ok(requested.max(total_documents))
}

/// Parameters for the `doc_put` RPC method.
#[derive(Debug, Deserialize)]
pub struct PutDocParams {
    /// Namespace to store the document in.
    pub namespace: String,
    /// Unique key for the document within the namespace.
    pub key: String,
    /// Human-readable title for the document.
    pub title: String,
    /// The raw text content of the document.
    pub content: String,
    /// The source type of the document (e.g., "doc", "web").
    #[serde(default = "default_source_type")]
    pub source_type: String,
    /// Priority level for retrieval (e.g., "high", "medium", "low").
    #[serde(default = "default_priority")]
    pub priority: String,
    /// Optional tags for categorization and filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Additional unstructured metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Core category for the document (e.g., "core", "user").
    #[serde(default = "default_category")]
    pub category: String,
    /// Optional session ID associated with the document.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional explicit document ID.
    #[serde(default)]
    pub document_id: Option<String>,
}

/// Parameters for the `doc_ingest` RPC method.
#[derive(Debug, Deserialize)]
pub struct IngestDocParams {
    /// Namespace to store the document in.
    pub namespace: String,
    /// Unique key for the document within the namespace.
    pub key: String,
    /// Human-readable title for the document.
    pub title: String,
    /// The raw text content of the document.
    pub content: String,
    /// The source type of the document.
    #[serde(default = "default_source_type")]
    pub source_type: String,
    /// Priority level for retrieval.
    #[serde(default = "default_priority")]
    pub priority: String,
    /// Optional tags for the document.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Additional unstructured metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Core category for the document.
    #[serde(default = "default_category")]
    pub category: String,
    /// Optional session ID.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional explicit document ID.
    #[serde(default)]
    pub document_id: Option<String>,
    /// Configuration for the ingestion process (chunking, etc.).
    #[serde(default)]
    pub config: Option<MemoryIngestionConfig>,
}

/// Parameters for RPC methods that only require a namespace.
#[derive(Debug, Deserialize)]
pub struct NamespaceOnlyParams {
    /// The target namespace.
    pub namespace: String,
}

/// Parameters for the `clear_namespace` RPC method.
#[derive(Debug, Deserialize)]
pub struct ClearNamespaceParams {
    /// The namespace to clear.
    pub namespace: String,
}

/// Result returned by the `clear_namespace` RPC method.
#[derive(Debug, Serialize)]
pub struct ClearNamespaceResult {
    /// Whether the namespace was successfully cleared.
    pub cleared: bool,
    /// The namespace that was cleared.
    pub namespace: String,
}

/// Parameters for the `doc_delete` RPC method.
#[derive(Debug, Deserialize)]
pub struct DeleteDocParams {
    /// The namespace containing the document.
    pub namespace: String,
    /// The unique ID of the document to delete.
    pub document_id: String,
}

/// Parameters for the `context_query` RPC method.
#[derive(Debug, Deserialize)]
pub struct QueryNamespaceParams {
    /// The namespace to query.
    pub namespace: String,
    /// The natural language query string.
    pub query: String,
    /// Maximum number of results to return.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters for the `context_recall` RPC method.
#[derive(Debug, Deserialize)]
pub struct RecallNamespaceParams {
    /// The namespace to recall from.
    pub namespace: String,
    /// Maximum number of results to return.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters for the `kv_set` RPC method.
#[derive(Debug, Deserialize)]
pub struct KvSetParams {
    /// The namespace for the key-value pair.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The unique key.
    pub key: String,
    /// The value to store.
    pub value: serde_json::Value,
}

/// Parameters for `kv_get` and `kv_delete` RPC methods.
#[derive(Debug, Deserialize)]
pub struct KvGetDeleteParams {
    /// The namespace containing the key.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The unique key.
    pub key: String,
}

/// Parameters for the `graph_upsert` RPC method.
#[derive(Debug, Deserialize)]
pub struct GraphUpsertParams {
    /// The namespace for the relation.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The subject of the relation triple.
    pub subject: String,
    /// The predicate (relationship) of the triple.
    pub predicate: String,
    /// The object of the triple.
    pub object: String,
    /// Additional attributes for the relation.
    #[serde(default)]
    pub attrs: serde_json::Value,
}

/// Parameters for the `graph_query` RPC method.
#[derive(Debug, Deserialize)]
pub struct GraphQueryParams {
    /// The namespace to query.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Optional subject filter.
    #[serde(default)]
    pub subject: Option<String>,
    /// Optional predicate filter.
    #[serde(default)]
    pub predicate: Option<String>,
}

/// Result returned by the `doc_put` RPC method.
#[derive(Debug, Serialize)]
pub struct PutDocResult {
    /// The unique ID of the upserted document.
    pub document_id: String,
}

fn default_source_type() -> String {
    "doc".to_string()
}

fn default_priority() -> String {
    "medium".to_string()
}

fn default_category() -> String {
    "core".to_string()
}

/// Lists all namespaces in the memory system.
pub async fn namespace_list() -> Result<RpcOutcome<Vec<String>>, String> {
    let client = active_memory_client().await?;
    let namespaces = client.list_namespaces().await?;
    Ok(RpcOutcome::single_log(
        namespaces,
        "memory namespaces listed",
    ))
}

/// Upserts a document into a namespace.
pub async fn doc_put(params: PutDocParams) -> Result<RpcOutcome<PutDocResult>, String> {
    let client = active_memory_client().await?;
    let document_id = client
        .put_doc(NamespaceDocumentInput {
            namespace: params.namespace,
            key: params.key,
            title: params.title,
            content: params.content,
            source_type: params.source_type,
            priority: params.priority,
            tags: params.tags,
            metadata: params.metadata,
            category: params.category,
            session_id: params.session_id,
            document_id: params.document_id,
        })
        .await?;
    Ok(RpcOutcome::single_log(
        PutDocResult { document_id },
        "memory document upserted",
    ))
}

/// Ingests a document, performing chunking and embedding.
pub async fn doc_ingest(
    params: IngestDocParams,
) -> Result<RpcOutcome<MemoryIngestionResult>, String> {
    let client = active_memory_client().await?;
    let result = client
        .ingest_doc(MemoryIngestionRequest {
            document: NamespaceDocumentInput {
                namespace: params.namespace,
                key: params.key,
                title: params.title,
                content: params.content,
                source_type: params.source_type,
                priority: params.priority,
                tags: params.tags,
                metadata: params.metadata,
                category: params.category,
                session_id: params.session_id,
                document_id: params.document_id,
            },
            config: params.config.unwrap_or_default(),
        })
        .await?;
    let msg = format!(
        "ingested document {} — {} entities, {} relations, {} chunks",
        result.document_id, result.entity_count, result.relation_count, result.chunk_count,
    );
    Ok(RpcOutcome::single_log(result, &msg))
}

/// Lists documents, optionally filtered by namespace.
pub async fn doc_list(
    params: Option<NamespaceOnlyParams>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let client = active_memory_client().await?;
    let docs = client
        .list_documents(params.as_ref().map(|v| v.namespace.as_str()))
        .await?;
    Ok(RpcOutcome::single_log(docs, "memory documents listed"))
}

/// Deletes a document from a namespace.
pub async fn doc_delete(params: DeleteDocParams) -> Result<RpcOutcome<serde_json::Value>, String> {
    let client = active_memory_client().await?;
    let result = client
        .delete_document(&params.namespace, &params.document_id)
        .await?;
    Ok(RpcOutcome::single_log(result, "memory document deleted"))
}

/// Clears all data within a namespace.
pub async fn clear_namespace(
    params: ClearNamespaceParams,
) -> Result<RpcOutcome<ClearNamespaceResult>, String> {
    let client = active_memory_client().await?;
    log::debug!(
        "[memory] clear_namespace RPC: namespace={}",
        params.namespace
    );
    client.clear_namespace(&params.namespace).await?;
    let msg = format!("memory namespace '{}' cleared", params.namespace);
    Ok(RpcOutcome::single_log(
        ClearNamespaceResult {
            cleared: true,
            namespace: params.namespace,
        },
        &msg,
    ))
}

/// Queries a namespace for contextual information based on a natural language string.
pub async fn context_query(params: QueryNamespaceParams) -> Result<RpcOutcome<String>, String> {
    let client = active_memory_client().await?;
    let result = client
        .query_namespace(&params.namespace, &params.query, params.limit.unwrap_or(10))
        .await?;
    Ok(RpcOutcome::single_log(result, "memory context queried"))
}

/// Recalls contextual information from a namespace without a specific query.
pub async fn context_recall(
    params: RecallNamespaceParams,
) -> Result<RpcOutcome<Option<String>>, String> {
    let client = active_memory_client().await?;
    let result = client
        .recall_namespace(&params.namespace, params.limit.unwrap_or(10))
        .await?;
    Ok(RpcOutcome::single_log(result, "memory context recalled"))
}

/// Sets a key-value pair in the memory store.
pub async fn kv_set(params: KvSetParams) -> Result<RpcOutcome<bool>, String> {
    let client = active_memory_client().await?;
    client
        .kv_set(params.namespace.as_deref(), &params.key, &params.value)
        .await?;
    Ok(RpcOutcome::single_log(true, "memory kv set"))
}

/// Retrieves a value by key from the memory store.
pub async fn kv_get(
    params: KvGetDeleteParams,
) -> Result<RpcOutcome<Option<serde_json::Value>>, String> {
    let client = active_memory_client().await?;
    let value = client
        .kv_get(params.namespace.as_deref(), &params.key)
        .await?;
    Ok(RpcOutcome::single_log(value, "memory kv get"))
}

/// Deletes a key-value pair from the memory store.
pub async fn kv_delete(params: KvGetDeleteParams) -> Result<RpcOutcome<bool>, String> {
    let client = active_memory_client().await?;
    let deleted = client
        .kv_delete(params.namespace.as_deref(), &params.key)
        .await?;
    Ok(RpcOutcome::single_log(deleted, "memory kv delete"))
}

/// Lists all key-value entries in a namespace.
pub async fn kv_list_namespace(
    params: NamespaceOnlyParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let client = active_memory_client().await?;
    let rows = client.kv_list_namespace(&params.namespace).await?;
    Ok(RpcOutcome::single_log(rows, "memory namespace kv listed"))
}

/// Upserts a relation triple in the knowledge graph.
pub async fn graph_upsert(params: GraphUpsertParams) -> Result<RpcOutcome<bool>, String> {
    let client = active_memory_client().await?;
    client
        .graph_upsert(
            params.namespace.as_deref(),
            &params.subject,
            &params.predicate,
            &params.object,
            &params.attrs,
        )
        .await?;
    Ok(RpcOutcome::single_log(true, "memory graph upserted"))
}

/// Queries relations from the knowledge graph.
pub async fn graph_query(
    params: GraphQueryParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let client = active_memory_client().await?;
    let rows = client
        .graph_query(
            params.namespace.as_deref(),
            params.subject.as_deref(),
            params.predicate.as_deref(),
        )
        .await?;
    Ok(RpcOutcome::single_log(rows, "memory graph queried"))
}

/// Initialise the local-only (SQLite) memory subsystem for the current workspace.
///
/// `request.jwt_token` is accepted for backward compatibility but ignored — all
/// memory operations are local.  Remote/cloud sync is a future consideration.
pub async fn memory_init(
    request: MemoryInitRequest,
) -> Result<RpcOutcome<ApiEnvelope<MemoryInitResponse>>, String> {
    let _ = request.jwt_token; // accepted but unused — memory is local-only
    let workspace_dir = current_workspace_dir().await?;
    // Initialise (or return existing) global singleton.
    let _ = super::global::init(workspace_dir.clone())?;
    let memory_dir = workspace_dir.join("memory");
    Ok(envelope(
        MemoryInitResponse {
            initialized: true,
            workspace_dir: workspace_dir.display().to_string(),
            memory_dir: memory_dir.display().to_string(),
        },
        None,
        None,
    ))
}

/// Lists documents stored in memory, optionally filtered by namespace.
pub async fn memory_list_documents(
    request: ListDocumentsRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListDocumentsResponse>>, String> {
    let client = active_memory_client().await?;
    let raw = client.list_documents(request.namespace.as_deref()).await?;
    let documents = parse_memory_document_summaries(raw)?;
    let count = documents.len();
    Ok(envelope(
        ListDocumentsResponse {
            namespace: request.namespace,
            documents,
            count,
        },
        Some(memory_counts([("num_documents", count)])),
        Some(PaginationMeta {
            limit: count,
            offset: 0,
            count,
        }),
    ))
}

/// Lists all namespaces that contain memory documents.
pub async fn memory_list_namespaces(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListNamespacesResponse>>, String> {
    let client = active_memory_client().await?;
    let namespaces = client.list_namespaces().await?;
    let count = namespaces.len();
    Ok(envelope(
        ListNamespacesResponse { namespaces, count },
        Some(memory_counts([("num_namespaces", count)])),
        None,
    ))
}

/// Deletes a specific document from a namespace.
pub async fn memory_delete_document(
    request: DeleteDocumentRequest,
) -> Result<RpcOutcome<ApiEnvelope<DeleteDocumentResponse>>, String> {
    let client = active_memory_client().await?;
    let raw = client
        .delete_document(&request.namespace, &request.document_id)
        .await?;
    let parsed: RawDeleteDocumentResult =
        serde_json::from_value(raw).map_err(|e| format!("decode delete document result: {e}"))?;
    Ok(envelope(
        DeleteDocumentResponse {
            status: if parsed.deleted {
                "completed".to_string()
            } else {
                "not_found".to_string()
            },
            namespace: parsed.namespace,
            document_id: parsed.document_id,
            deleted: parsed.deleted,
        },
        None,
        None,
    ))
}

/// Performs a semantic query against a namespace, returning a retrieval context.
pub async fn memory_query_namespace(
    request: QueryNamespaceRequest,
) -> Result<RpcOutcome<ApiEnvelope<QueryNamespaceResponse>>, String> {
    let include_references = request.include_references.unwrap_or(true);
    let result = async {
        let client = active_memory_client().await?;
        let retrieval_limit = query_limit_for_request(client.as_ref(), &request).await?;
        let mut context = client
            .query_namespace_context_data(&request.namespace, &request.query, retrieval_limit)
            .await?;
        context.hits = filter_hits_by_document_ids(context.hits, request.document_ids.as_deref());
        Ok::<NamespaceRetrievalContext, String>(context)
    }
    .await;

    match result {
        Ok(context) => {
            let retrieval_context = build_retrieval_context(&context.hits);
            let counts = memory_counts([
                ("num_entities", retrieval_context.entities.len()),
                ("num_relations", retrieval_context.relations.len()),
                ("num_chunks", retrieval_context.chunks.len()),
            ]);
            let llm_context_message =
                format_llm_context_message(Some(&request.query), &context.hits);
            Ok(envelope(
                QueryNamespaceResponse {
                    context: maybe_retrieval_context(include_references, retrieval_context),
                    llm_context_message,
                },
                Some(counts),
                None,
            ))
        }
        Err(message) => Ok(error_envelope("memory.query_namespace_failed", message)),
    }
}

/// Recalls contextual data from a namespace without a specific query.
pub async fn memory_recall_context(
    request: RecallContextRequest,
) -> Result<RpcOutcome<ApiEnvelope<RecallContextResponse>>, String> {
    let include_references = request.include_references.unwrap_or(true);
    let result = async {
        let client = active_memory_client().await?;
        client
            .recall_namespace_context_data(&request.namespace, request.resolved_limit())
            .await
    }
    .await;

    match result {
        Ok(context) => {
            let retrieval_context = build_retrieval_context(&context.hits);
            let counts = memory_counts([
                ("num_entities", retrieval_context.entities.len()),
                ("num_relations", retrieval_context.relations.len()),
                ("num_chunks", retrieval_context.chunks.len()),
            ]);
            let llm_context_message = format_llm_context_message(None, &context.hits);
            Ok(envelope(
                RecallContextResponse {
                    context: maybe_retrieval_context(include_references, retrieval_context),
                    llm_context_message,
                },
                Some(counts),
                None,
            ))
        }
        Err(message) => Ok(error_envelope("memory.recall_context_failed", message)),
    }
}

/// Recalls memory items from a namespace with optional retention filtering.
pub async fn memory_recall_memories(
    request: RecallMemoriesRequest,
) -> Result<RpcOutcome<ApiEnvelope<RecallMemoriesResponse>>, String> {
    let result = async {
        let client = active_memory_client().await?;
        client
            .recall_namespace_memories(&request.namespace, request.resolved_limit())
            .await
    }
    .await;

    match result {
        Ok(hits) => {
            let memories = hits
                .into_iter()
                .map(|hit| MemoryRecallItem {
                    kind: memory_kind_label(&hit.kind).to_string(),
                    id: hit.id,
                    content: hit.content,
                    score: hit.score,
                    retention: None,
                    last_accessed_at: None,
                    access_count: None,
                    stability_days: None,
                })
                .collect::<Vec<_>>();
            let count = memories.len();
            Ok(envelope(
                RecallMemoriesResponse { memories },
                Some(memory_counts([("num_memories", count)])),
                None,
            ))
        }
        Err(message) => Ok(error_envelope("memory.recall_memories_failed", message)),
    }
}

/// Lists files in a memory directory.
pub async fn ai_list_memory_files(
    request: ListMemoryFilesRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListMemoryFilesResponse>>, String> {
    validate_memory_relative_path(&request.relative_dir)?;
    let directory = resolve_existing_memory_path(&request.relative_dir).await?;
    if !directory.is_dir() {
        return Err(format!(
            "memory directory not found: {}",
            directory.display()
        ));
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&directory)
        .map_err(|e| format!("read memory directory {}: {e}", directory.display()))?
    {
        let entry = entry.map_err(|e| format!("read memory directory entry: {e}"))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.is_empty() {
            files.push(file_name.to_string());
        }
    }
    files.sort();
    let count = files.len();
    Ok(envelope(
        ListMemoryFilesResponse {
            relative_dir: request.relative_dir,
            files,
            count,
        },
        Some(memory_counts([("num_files", count)])),
        None,
    ))
}

/// Reads the contents of a memory file.
pub async fn ai_read_memory_file(
    request: ReadMemoryFileRequest,
) -> Result<RpcOutcome<ApiEnvelope<ReadMemoryFileResponse>>, String> {
    let path = resolve_existing_memory_path(&request.relative_path).await?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read memory file {}: {e}", path.display()))?;
    Ok(envelope(
        ReadMemoryFileResponse {
            relative_path: request.relative_path,
            content,
        },
        None,
        None,
    ))
}

/// Writes content to a memory file.
pub async fn ai_write_memory_file(
    request: WriteMemoryFileRequest,
) -> Result<RpcOutcome<ApiEnvelope<WriteMemoryFileResponse>>, String> {
    let path = resolve_writable_memory_path(&request.relative_path).await?;
    std::fs::write(&path, request.content.as_bytes())
        .map_err(|e| format!("write memory file {}: {e}", path.display()))?;
    let bytes_written = request.content.len();
    Ok(envelope(
        WriteMemoryFileResponse {
            relative_path: request.relative_path,
            written: true,
            bytes_written,
        },
        None,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_retrieval_context, filter_hits_by_document_ids, format_llm_context_message};
    use crate::openhuman::memory::store::GraphRelationRecord;
    use crate::openhuman::memory::{MemoryItemKind, NamespaceMemoryHit, RetrievalScoreBreakdown};

    fn sample_hit() -> NamespaceMemoryHit {
        NamespaceMemoryHit {
            id: "doc-1".to_string(),
            kind: MemoryItemKind::Document,
            namespace: "team".to_string(),
            key: "atlas-status".to_string(),
            title: Some("Atlas status".to_string()),
            content: "Project Atlas is owned by Alice.".to_string(),
            category: "core".to_string(),
            source_type: Some("doc".to_string()),
            updated_at: 1_700_000_000.0,
            score: 0.92,
            score_breakdown: RetrievalScoreBreakdown {
                keyword_relevance: 0.3,
                vector_similarity: 0.4,
                graph_relevance: 0.9,
                episodic_relevance: 0.0,
                freshness: 0.0,
                final_score: 0.92,
            },
            document_id: Some("doc-1".to_string()),
            chunk_id: Some("doc-1#chunk-1".to_string()),
            supporting_relations: vec![GraphRelationRecord {
                namespace: Some("team".to_string()),
                subject: "Alice".to_string(),
                predicate: "OWNS".to_string(),
                object: "Atlas".to_string(),
                attrs: json!({"source": "graph"}),
                updated_at: 1_700_000_000.0,
                evidence_count: 2,
                order_index: Some(1),
                document_ids: vec!["doc-1".to_string()],
                chunk_ids: vec!["doc-1#chunk-1".to_string()],
            }],
        }
    }

    #[test]
    fn build_retrieval_context_projects_hits_into_relations_and_chunks() {
        let context = build_retrieval_context(&[sample_hit()]);
        assert_eq!(context.entities.len(), 2);
        assert_eq!(context.relations.len(), 1);
        assert_eq!(context.chunks.len(), 1);
        assert_eq!(context.chunks[0].document_id.as_deref(), Some("doc-1"));
        assert_eq!(context.relations[0].predicate, "OWNS");
    }

    fn sample_hit_with_entity_types() -> NamespaceMemoryHit {
        NamespaceMemoryHit {
            id: "doc-2".to_string(),
            kind: MemoryItemKind::Document,
            namespace: "team".to_string(),
            key: "atlas-status".to_string(),
            title: Some("Atlas status".to_string()),
            content: "Project Atlas is owned by Alice.".to_string(),
            category: "core".to_string(),
            source_type: Some("doc".to_string()),
            updated_at: 1_700_000_000.0,
            score: 0.92,
            score_breakdown: RetrievalScoreBreakdown {
                keyword_relevance: 0.3,
                vector_similarity: 0.4,
                graph_relevance: 0.9,
                episodic_relevance: 0.0,
                freshness: 0.0,
                final_score: 0.92,
            },
            document_id: Some("doc-2".to_string()),
            chunk_id: Some("doc-2#chunk-1".to_string()),
            supporting_relations: vec![GraphRelationRecord {
                namespace: Some("team".to_string()),
                subject: "Alice".to_string(),
                predicate: "OWNS".to_string(),
                object: "Atlas".to_string(),
                attrs: json!({
                    "source": "ingestion",
                    "entity_types": {
                        "subject": "PERSON",
                        "object": "PROJECT"
                    }
                }),
                updated_at: 1_700_000_000.0,
                evidence_count: 2,
                order_index: Some(1),
                document_ids: vec!["doc-2".to_string()],
                chunk_ids: vec!["doc-2#chunk-1".to_string()],
            }],
        }
    }

    #[test]
    fn build_retrieval_context_extracts_entity_types_from_attrs() {
        let context = build_retrieval_context(&[sample_hit_with_entity_types()]);
        assert_eq!(context.entities.len(), 2);

        let alice = context.entities.iter().find(|e| e.name == "Alice").unwrap();
        assert_eq!(alice.entity_type.as_deref(), Some("PERSON"));

        let atlas = context.entities.iter().find(|e| e.name == "Atlas").unwrap();
        assert_eq!(atlas.entity_type.as_deref(), Some("PROJECT"));
    }

    #[test]
    fn build_retrieval_context_entity_type_none_when_attrs_missing() {
        let context = build_retrieval_context(&[sample_hit()]);
        assert_eq!(context.entities.len(), 2);

        for entity in &context.entities {
            assert_eq!(
                entity.entity_type, None,
                "entity_type should be None when attrs has no entity_types"
            );
        }
    }

    #[test]
    fn helpers_filter_document_ids_and_format_context_message() {
        let hit = sample_hit();
        let filtered = filter_hits_by_document_ids(vec![hit.clone()], Some(&["doc-2".to_string()]));
        assert!(filtered.is_empty());

        let message = format_llm_context_message(Some("who owns atlas"), &[hit])
            .expect("context message should exist");
        assert!(message.contains("Query: who owns atlas"));
        // Without entity_types in attrs, relations render without type annotations.
        assert!(message.contains("Alice -[OWNS]-> Atlas"));
    }

    #[test]
    fn format_llm_context_message_includes_entity_types_when_present() {
        let hit = sample_hit_with_entity_types();
        let message = format_llm_context_message(Some("who owns atlas"), &[hit])
            .expect("context message should exist");
        assert!(
            message.contains("Alice (PERSON) -[OWNS]-> Atlas (PROJECT)"),
            "expected entity types in relation text, got: {message}"
        );
    }

    // ── Pure-helper coverage ───────────────────────────────────────

    use super::{
        chunk_metadata, default_category, default_priority, default_source_type, error_envelope,
        extract_entity_type, maybe_retrieval_context, memory_counts, memory_kind_label,
        memory_request_id, relation_identity, relation_metadata, timestamp_to_rfc3339,
        validate_memory_relative_path,
    };
    use crate::openhuman::memory::{ApiEnvelope, MemoryRetrievalContext};
    use crate::rpc::RpcOutcome;

    #[test]
    fn memory_request_id_is_nonempty_and_unique() {
        let a = memory_request_id();
        let b = memory_request_id();
        assert!(!a.is_empty());
        assert!(!b.is_empty());
        assert_ne!(a, b);
    }

    #[test]
    fn memory_counts_builds_btreemap_from_entries() {
        let m = memory_counts([("documents", 3), ("kv", 1)]);
        assert_eq!(m.get("documents"), Some(&3));
        assert_eq!(m.get("kv"), Some(&1));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn memory_counts_is_empty_for_empty_input() {
        let m: std::collections::BTreeMap<String, usize> = memory_counts(std::iter::empty());
        assert!(m.is_empty());
    }

    #[test]
    fn timestamp_to_rfc3339_valid_seconds_and_fractional() {
        let s = timestamp_to_rfc3339(1_700_000_000.0).unwrap();
        assert!(s.contains("2023"));
        // Fractional seconds should preserve nanoseconds within range.
        let s = timestamp_to_rfc3339(1_700_000_000.5).unwrap();
        assert!(s.contains("2023"));
    }

    #[test]
    fn timestamp_to_rfc3339_rejects_non_finite_and_negative() {
        assert!(timestamp_to_rfc3339(f64::NAN).is_none());
        assert!(timestamp_to_rfc3339(f64::INFINITY).is_none());
        assert!(timestamp_to_rfc3339(-1.0).is_none());
    }

    #[test]
    fn memory_kind_label_maps_each_variant() {
        assert_eq!(memory_kind_label(&MemoryItemKind::Document), "document");
        assert_eq!(memory_kind_label(&MemoryItemKind::Kv), "kv");
        assert_eq!(memory_kind_label(&MemoryItemKind::Episodic), "episodic");
        assert_eq!(memory_kind_label(&MemoryItemKind::Event), "event");
    }

    fn relation_fixture(namespace: Option<&str>) -> GraphRelationRecord {
        GraphRelationRecord {
            namespace: namespace.map(str::to_string),
            subject: "Alice".into(),
            predicate: "OWNS".into(),
            object: "Atlas".into(),
            attrs: json!({"entity_types":{"subject":"PERSON","object":"PROJECT"}}),
            updated_at: 1_700_000_000.0,
            evidence_count: 2,
            order_index: Some(1),
            document_ids: vec!["doc-1".into()],
            chunk_ids: vec!["doc-1#c1".into()],
        }
    }

    #[test]
    fn relation_identity_uses_global_for_missing_namespace() {
        let rel = relation_fixture(None);
        assert_eq!(relation_identity(&rel), "global|Alice|OWNS|Atlas");
        let rel = relation_fixture(Some("team"));
        assert_eq!(relation_identity(&rel), "team|Alice|OWNS|Atlas");
    }

    #[test]
    fn relation_metadata_includes_expected_keys() {
        let rel = relation_fixture(Some("team"));
        let m = relation_metadata(&rel);
        assert_eq!(m["namespace"], "team");
        assert_eq!(m["order_index"], 1);
        assert!(m["document_ids"].is_array());
        assert!(m["updated_at"].is_string());
    }

    #[test]
    fn chunk_metadata_exposes_score_breakdown() {
        let m = chunk_metadata(&sample_hit());
        assert_eq!(m["kind"], "document");
        assert_eq!(m["namespace"], "team");
        assert!(m["score_breakdown"]["final_score"].is_number());
    }

    #[test]
    fn extract_entity_type_returns_nonempty_or_none() {
        let attrs = json!({"entity_types":{"subject":"PERSON","object":""}});
        assert_eq!(
            extract_entity_type(&attrs, "subject"),
            Some("PERSON".into())
        );
        // Empty string → None.
        assert_eq!(extract_entity_type(&attrs, "object"), None);
        // Missing role → None.
        assert_eq!(extract_entity_type(&attrs, "missing"), None);
        // Empty attrs → None.
        assert_eq!(extract_entity_type(&json!({}), "subject"), None);
    }

    #[test]
    fn format_llm_context_message_returns_none_for_empty_hits() {
        assert!(format_llm_context_message(None, &[]).is_none());
        assert!(format_llm_context_message(Some("query"), &[]).is_none());
    }

    #[test]
    fn filter_hits_by_document_ids_passes_through_when_filter_is_none() {
        let hits = vec![sample_hit()];
        let filtered = filter_hits_by_document_ids(hits.clone(), None);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn filter_hits_by_document_ids_retains_matching_ids() {
        let hits = vec![sample_hit()];
        let filtered = filter_hits_by_document_ids(hits, Some(&["doc-1".to_string()]));
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn maybe_retrieval_context_respects_include_flag() {
        let empty = MemoryRetrievalContext {
            entities: vec![],
            relations: vec![],
            chunks: vec![],
        };
        // include=false → always None
        assert!(maybe_retrieval_context(false, empty.clone()).is_none());
        // include=true but context empty → None
        assert!(maybe_retrieval_context(true, empty).is_none());
        // include=true + non-empty context → Some
        let ctx = build_retrieval_context(&[sample_hit()]);
        assert!(maybe_retrieval_context(true, ctx).is_some());
    }

    #[test]
    fn default_constants_are_stable() {
        assert!(!default_source_type().is_empty());
        assert!(!default_priority().is_empty());
        assert!(!default_category().is_empty());
    }

    #[test]
    fn validate_memory_relative_path_rejects_empty_absolute_and_traversal() {
        assert!(validate_memory_relative_path("").is_err());
        assert!(validate_memory_relative_path("/etc/passwd").is_err());
        assert!(validate_memory_relative_path("../secrets").is_err());
        assert!(validate_memory_relative_path("ok/subdir/file.md").is_ok());
        assert!(validate_memory_relative_path("simple.txt").is_ok());
    }

    #[test]
    fn error_envelope_produces_api_error_with_code_and_message() {
        let envelope: RpcOutcome<ApiEnvelope<serde_json::Value>> =
            error_envelope::<serde_json::Value>("NOT_FOUND", "missing".into());
        let api = &envelope.value;
        assert!(api.data.is_none());
        let err = api.error.as_ref().expect("error set");
        assert_eq!(err.code, "NOT_FOUND");
        assert_eq!(err.message, "missing");
        // Meta must carry a request id.
        assert!(!api.meta.request_id.is_empty());
    }
}
