//! Document, namespace, and recall RPC handlers — both the unified-memory
//! direct API (`doc_*`, `namespace_*`, `context_*`) and the envelope-style
//! façade (`memory_init`, `memory_list_documents`, `memory_query_namespace`,
//! `memory_recall_*`).

use serde::{Deserialize, Serialize};

use crate::openhuman::memory::{
    ApiEnvelope, DeleteDocumentRequest, DeleteDocumentResponse, EmptyRequest, ListDocumentsRequest,
    ListDocumentsResponse, ListNamespacesResponse, MemoryIngestionConfig, MemoryIngestionRequest,
    MemoryIngestionResult, MemoryInitRequest, MemoryInitResponse, MemoryRecallItem,
    NamespaceDocumentInput, NamespaceRetrievalContext, PaginationMeta, QueryNamespaceRequest,
    QueryNamespaceResponse, RecallContextRequest, RecallContextResponse, RecallMemoriesRequest,
    RecallMemoriesResponse,
};
use crate::rpc::RpcOutcome;

use super::envelope::{envelope, error_envelope, memory_counts};
use super::helpers::{
    active_memory_client, build_retrieval_context, current_workspace_dir,
    filter_hits_by_document_ids, format_llm_context_message, maybe_retrieval_context,
    memory_kind_label, parse_memory_document_summaries, query_limit_for_request,
    RawDeleteDocumentResult,
};
use super::helpers::{default_category, default_priority, default_source_type};

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

/// Result returned by the `doc_put` RPC method.
#[derive(Debug, Serialize)]
pub struct PutDocResult {
    /// The unique ID of the upserted document.
    pub document_id: String,
}

// ---------------------------------------------------------------------------
// Unified-memory direct API
// ---------------------------------------------------------------------------

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
        "ingested document — {} entities, {} relations, {} chunks",
        result.entity_count, result.relation_count, result.chunk_count,
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
    log::debug!("[memory] clear_namespace RPC invoked");
    client.clear_namespace(&params.namespace).await?;
    let msg = "memory namespace cleared".to_string();
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

// ---------------------------------------------------------------------------
// Envelope-style façade (`memory_*`)
// ---------------------------------------------------------------------------

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
    let _ = super::super::global::init(workspace_dir.clone())?;
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
    let requested_limit = request.resolved_limit() as usize;
    let result = async {
        let client = active_memory_client().await?;
        let retrieval_limit = query_limit_for_request(client.as_ref(), &request).await?;
        let mut context = client
            .query_namespace_context_data(&request.namespace, &request.query, retrieval_limit)
            .await?;
        context.hits = filter_hits_by_document_ids(context.hits, request.document_ids.as_deref());
        // `query_limit_for_request` may have over-fetched on purpose so that
        // the document_id filter has enough candidates; truncate back to what
        // the caller actually asked for.
        if context.hits.len() > requested_limit {
            context.hits.truncate(requested_limit);
        }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn ensure_memory_client() {
        static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
        let workspace = WORKSPACE.get_or_init(|| {
            let tmp = TempDir::new().expect("tempdir");
            let path = tmp.path().join("workspace");
            std::fs::create_dir_all(&path).expect("workspace dir");
            std::mem::forget(tmp);
            path
        });
        let _ = crate::openhuman::memory::global::init(workspace.clone());
    }

    fn unique_namespace(prefix: &str) -> String {
        format!("{prefix}-{}", uuid::Uuid::new_v4())
    }

    fn sample_put(namespace: String, key: String, title: &str, content: &str) -> PutDocParams {
        PutDocParams {
            namespace,
            key,
            title: title.into(),
            content: content.into(),
            source_type: default_source_type(),
            priority: default_priority(),
            tags: vec!["test".into()],
            metadata: json!({"source": "test"}),
            category: default_category(),
            session_id: Some("session-docs".into()),
            document_id: None,
        }
    }

    #[tokio::test]
    async fn direct_document_handlers_roundtrip_through_namespace() {
        ensure_memory_client();
        let namespace = unique_namespace("memory-docs-direct");
        let key = format!("note-{}", uuid::Uuid::new_v4());

        let put = doc_put(sample_put(
            namespace.clone(),
            key.clone(),
            "Rust ownership",
            "Ownership and borrowing let Rust enforce memory safety.",
        ))
        .await
        .expect("doc_put");
        let document_id = put.value.document_id.clone();
        assert!(!document_id.is_empty());

        let listed = doc_list(Some(NamespaceOnlyParams {
            namespace: namespace.clone(),
        }))
        .await
        .expect("doc_list");
        let docs = listed
            .value
            .get("documents")
            .and_then(|v| v.as_array())
            .expect("documents array");
        assert!(docs.iter().any(|doc| doc["key"] == key));

        let queried = context_query(QueryNamespaceParams {
            namespace: namespace.clone(),
            query: "ownership".into(),
            limit: Some(5),
        })
        .await
        .expect("context_query");
        assert!(
            queried.value.to_lowercase().contains("ownership"),
            "query result should mention the stored concept"
        );

        let recalled = context_recall(RecallNamespaceParams {
            namespace: namespace.clone(),
            limit: Some(5),
        })
        .await
        .expect("context_recall");
        assert!(recalled.value.is_some());

        let deleted = doc_delete(DeleteDocParams {
            namespace: namespace.clone(),
            document_id: document_id.clone(),
        })
        .await
        .expect("doc_delete");
        assert_eq!(deleted.logs, vec!["memory document deleted".to_string()]);

        let deleted_flag = deleted
            .value
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(deleted_flag, "delete result should report success");

        let after = doc_list(Some(NamespaceOnlyParams { namespace }))
            .await
            .expect("doc_list after delete");
        let after_docs = after
            .value
            .get("documents")
            .and_then(|v| v.as_array())
            .expect("documents array after delete");
        assert!(after_docs.is_empty());
    }

    #[tokio::test]
    async fn envelope_memory_handlers_report_counts_and_statuses() {
        ensure_memory_client();
        let namespace = unique_namespace("memory-docs-envelope");
        let key = format!("env-{}", uuid::Uuid::new_v4());

        let _ = memory_init(MemoryInitRequest { jwt_token: None })
            .await
            .expect("memory_init");

        let direct = doc_put(sample_put(
            namespace.clone(),
            key.clone(),
            "Borrow checker",
            "The borrow checker enforces aliasing and mutation rules.",
        ))
        .await
        .expect("seed document");
        let document_id = direct.value.document_id;

        let listed = memory_list_documents(ListDocumentsRequest {
            namespace: Some(namespace.clone()),
        })
        .await
        .expect("memory_list_documents");
        let listed_data = listed.value.data.expect("list envelope data");
        assert_eq!(listed_data.count, 1);
        assert_eq!(listed_data.documents[0].key, key);
        assert_eq!(
            listed
                .value
                .meta
                .counts
                .as_ref()
                .and_then(|m| m.get("num_documents")),
            Some(&1)
        );

        let namespaces = memory_list_namespaces(EmptyRequest {})
            .await
            .expect("memory_list_namespaces");
        let namespace_data = namespaces.value.data.expect("namespace data");
        assert!(
            namespace_data.namespaces.iter().any(|ns| ns == &namespace),
            "expected namespace list to include the seeded namespace"
        );

        let queried = memory_query_namespace(QueryNamespaceRequest {
            namespace: namespace.clone(),
            query: "borrow checker".into(),
            limit: Some(5),
            max_chunks: None,
            include_references: Some(true),
            document_ids: None,
        })
        .await
        .expect("memory_query_namespace");
        let query_data = queried.value.data.expect("query data");
        assert!(query_data.llm_context_message.is_some());
        assert!(query_data.context.is_some());

        let recalled = memory_recall_memories(RecallMemoriesRequest {
            namespace: namespace.clone(),
            min_retention: None,
            as_of: None,
            limit: Some(5),
            max_chunks: None,
            top_k: None,
        })
        .await
        .expect("memory_recall_memories");
        let recall_data = recalled.value.data.expect("recall data");
        assert_eq!(recall_data.memories.len(), 1);
        assert_eq!(recall_data.memories[0].kind, "document");

        let deleted = memory_delete_document(DeleteDocumentRequest {
            namespace: namespace.clone(),
            document_id,
        })
        .await
        .expect("memory_delete_document");
        let deleted_data = deleted.value.data.expect("delete envelope data");
        assert_eq!(deleted_data.status, "completed");
        assert!(deleted_data.deleted);

        let cleared = clear_namespace(ClearNamespaceParams {
            namespace: namespace.clone(),
        })
        .await
        .expect("clear_namespace");
        assert!(cleared.value.cleared);

        let listed_after = memory_list_documents(ListDocumentsRequest {
            namespace: Some(namespace),
        })
        .await
        .expect("memory_list_documents after clear");
        let after_data = listed_after.value.data.expect("after clear data");
        assert_eq!(after_data.count, 0);
    }
}
