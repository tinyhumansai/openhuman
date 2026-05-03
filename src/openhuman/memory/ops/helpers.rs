//! Formatting helpers, default constants, path validators, and the active
//! memory-client lookup. Shared internals for the memory RPC handlers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use chrono::TimeZone;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::memory::store::GraphRelationRecord;
use crate::openhuman::memory::{
    MemoryClient, MemoryClientRef, MemoryDocumentSummary, MemoryItemKind, MemoryRetrievalChunk,
    MemoryRetrievalContext, MemoryRetrievalEntity, MemoryRetrievalRelation, NamespaceMemoryHit,
    QueryNamespaceRequest,
};

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Formats a floating-point timestamp as an RFC3339 string.
///
/// Returns `None` if the timestamp is invalid (NaN, infinite, or negative).
pub(crate) fn timestamp_to_rfc3339(timestamp: f64) -> Option<String> {
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
pub(crate) fn memory_kind_label(kind: &MemoryItemKind) -> &'static str {
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
pub(crate) fn relation_identity(relation: &GraphRelationRecord) -> String {
    format!(
        "{}|{}|{}|{}",
        relation.namespace.as_deref().unwrap_or("global"),
        relation.subject.as_str(),
        relation.predicate.as_str(),
        relation.object.as_str()
    )
}

/// Formats relation metadata into a JSON Value.
pub(crate) fn relation_metadata(relation: &GraphRelationRecord) -> Value {
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
pub(crate) fn chunk_metadata(hit: &NamespaceMemoryHit) -> Value {
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
pub(crate) fn extract_entity_type(attrs: &Value, role: &str) -> Option<String> {
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
pub(crate) fn format_llm_context_message(
    query: Option<&str>,
    hits: &[NamespaceMemoryHit],
) -> Option<String> {
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
pub(crate) fn filter_hits_by_document_ids(
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
pub(crate) fn maybe_retrieval_context(
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

// ---------------------------------------------------------------------------
// Default constants
// ---------------------------------------------------------------------------

pub(crate) fn default_source_type() -> String {
    "doc".to_string()
}

pub(crate) fn default_priority() -> String {
    "medium".to_string()
}

pub(crate) fn default_category() -> String {
    "core".to_string()
}

// ---------------------------------------------------------------------------
// Workspace + memory-client lookup
// ---------------------------------------------------------------------------

/// Subdirectory under the workspace where the file-based memory RPCs operate.
/// `ai_*_memory_file` handlers MUST resolve all caller-supplied relative paths
/// against this directory — never the workspace root — to avoid leaking access
/// to repo files such as `Cargo.toml`, `.env`, or source files.
const MEMORY_SUBDIR: &str = "memory";

/// Returns the current workspace directory from configuration.
pub(crate) async fn current_workspace_dir() -> Result<PathBuf, String> {
    Config::load_or_init()
        .await
        .map(|config| config.workspace_dir)
        .map_err(|e| format!("load config: {e}"))
}

/// Returns the active memory client from the process-global singleton,
/// auto-initialising from the configured workspace if startup wiring hasn't
/// done so yet.
///
/// The auto-init resolves the workspace via [`current_workspace_dir`], which
/// goes through `Config::load_or_init` — the same path startup wiring uses.
/// It does **not** fall back to `~/.openhuman/workspace`; that hazard is the
/// one [`crate::openhuman::memory::global::client`] guards against, and it
/// remains guarded for any caller that bypasses this helper.
pub(crate) async fn active_memory_client() -> Result<MemoryClientRef, String> {
    if let Some(client) = super::super::global::client_if_ready() {
        return Ok(client);
    }
    let workspace_dir = current_workspace_dir().await?;
    super::super::global::init(workspace_dir)
}

// ---------------------------------------------------------------------------
// Path validators (used by file-based memory handlers)
// ---------------------------------------------------------------------------

/// Validates that a relative path does not escape the memory directory.
///
/// An empty path is allowed and refers to the memory root itself
/// (`<workspace>/memory`); read-style helpers can resolve it to that
/// directory. Write helpers reject empty paths separately because they
/// require a file name component.
pub(crate) fn validate_memory_relative_path(path: &str) -> Result<(), String> {
    let candidate = Path::new(path);
    if candidate.as_os_str().is_empty() {
        return Ok(());
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
pub(crate) async fn resolve_memory_root() -> Result<PathBuf, String> {
    let workspace_dir = current_workspace_dir().await?;
    let memory_root = workspace_dir.join(MEMORY_SUBDIR);
    tokio::fs::create_dir_all(&memory_root)
        .await
        .map_err(|e| format!("create memory dir {}: {e}", memory_root.display()))?;
    memory_root
        .canonicalize()
        .map_err(|e| format!("resolve memory dir {}: {e}", memory_root.display()))
}

/// Resolves and canonicalizes an existing memory path, ensuring it stays within
/// the `<workspace>/memory` directory (not the workspace root). An empty
/// `relative_path` resolves to the memory root itself.
pub(crate) async fn resolve_existing_memory_path(relative_path: &str) -> Result<PathBuf, String> {
    validate_memory_relative_path(relative_path)?;
    let memory_root = resolve_memory_root().await?;
    let full_path = if relative_path.is_empty() {
        memory_root.clone()
    } else {
        memory_root.join(relative_path)
    };
    let resolved = full_path
        .canonicalize()
        .map_err(|e| format!("resolve memory path {}: {e}", full_path.display()))?;
    if !resolved.starts_with(&memory_root) {
        return Err("memory path escapes the memory directory".to_string());
    }
    Ok(resolved)
}

/// Resolves a path for writing, creating parent directories and ensuring it
/// stays within the `<workspace>/memory` directory (not the workspace root).
pub(crate) async fn resolve_writable_memory_path(relative_path: &str) -> Result<PathBuf, String> {
    validate_memory_relative_path(relative_path)?;
    let memory_root = resolve_memory_root().await?;
    let full_path = memory_root.join(relative_path);
    let parent = full_path
        .parent()
        .ok_or_else(|| "memory path must include a file name".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("create memory path {}: {e}", parent.display()))?;
    let resolved_parent = parent
        .canonicalize()
        .map_err(|e| format!("resolve memory parent {}: {e}", parent.display()))?;
    if !resolved_parent.starts_with(&memory_root) {
        return Err("memory path escapes the memory directory".to_string());
    }
    let file_name = full_path
        .file_name()
        .ok_or_else(|| "memory path must include a file name".to_string())?;
    let resolved = resolved_parent.join(file_name);
    // Security check: refuse to write through symlinks to prevent hijacking
    if let Ok(metadata) = tokio::fs::symlink_metadata(&resolved).await {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "refusing to write through symlink: {}",
                resolved.display()
            ));
        }
    }
    Ok(resolved)
}

// ---------------------------------------------------------------------------
// Document summary parsing + query-limit resolution (shared by documents.rs)
// ---------------------------------------------------------------------------

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
pub(crate) struct RawDeleteDocumentResult {
    pub deleted: bool,
    pub namespace: String,
    pub document_id: String,
}

pub(crate) fn parse_memory_document_summaries(
    raw: Value,
) -> Result<Vec<MemoryDocumentSummary>, String> {
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

pub(crate) async fn query_limit_for_request(
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
