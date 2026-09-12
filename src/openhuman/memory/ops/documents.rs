//! Document, namespace, and recall RPC handlers — both the unified-memory
//! direct API (`doc_*`, `namespace_*`, `context_*`) and the envelope-style
//! façade (`memory_init`, `memory_list_documents`, `memory_query_namespace`,
//! `memory_recall_*`).

use serde::{Deserialize, Serialize};

use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::api::types::NamespaceDocumentInput;
use crate::openhuman::memory::api::types::NamespaceRetrievalContext;
use crate::openhuman::memory::{
    ApiEnvelope, DeleteDocumentRequest, DeleteDocumentResponse, EmptyRequest, ListDocumentsRequest,
    ListDocumentsResponse, ListNamespacesResponse, MemoryIngestionConfig, MemoryIngestionResult,
    MemoryInitRequest, MemoryInitResponse, MemoryRecallItem, PaginationMeta, QueryNamespaceRequest,
    QueryNamespaceResponse, RecallContextRequest, RecallContextResponse, RecallMemoriesRequest,
    RecallMemoriesResponse,
};
use crate::rpc::RpcOutcome;

use super::envelope::{envelope, error_envelope, memory_counts};
use super::guard::active_memory_guard;
use super::helpers::{
    build_retrieval_context, current_workspace_dir, filter_hits_by_document_ids,
    format_llm_context_message, maybe_retrieval_context, memory_kind_label,
    parse_memory_document_summaries, query_limit_for_request, RawDeleteDocumentResult,
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
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let namespaces = documents
        .list_namespaces()
        .await
        .map_err(|error| error.to_string())?;
    Ok(RpcOutcome::single_log(
        namespaces,
        "memory namespaces listed",
    ))
}

/// Upserts a document into a namespace.
///
/// Routed through [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard).
/// `MemoryDocuments::put_document` on the embedded driver is
/// `client.put_doc(input)` — deliberately the full pipeline, not the
/// `put_doc_light` shortcut — so the store, the input type and the background
/// graph-extraction enqueue are all unchanged. What the guard adds: the tier
/// check, redaction (a byte-identical pass-through for an embedded driver), and
/// taint stamping.
///
/// The `taint: Internal` literal below stays: the contract says the *caller*
/// supplies provenance and the driver never assigns it. `GuardPolicy::stamp_taint`
/// is a monotone raise over that value — it can promote this write to
/// `ExternalSync` when the turn runs under a source scope, but it can never
/// launder an `ExternalSync` caller down to `Internal`.
pub async fn doc_put(params: PutDocParams) -> Result<RpcOutcome<PutDocResult>, String> {
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let document_id = documents
        .put_document(NamespaceDocumentInput {
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
            // RPC-driven doc puts come from the user / agent — Internal.
            // External-sync ingest paths bypass this RPC and call
            // `store_skill_sync` directly with their own taint label.
            taint: crate::openhuman::memory::api::types::MemoryTaint::Internal,
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        PutDocResult { document_id },
        "memory document upserted",
    ))
}

/// Ingests a document, performing chunking and embedding.
pub async fn doc_ingest(
    params: IngestDocParams,
) -> Result<RpcOutcome<MemoryIngestionResult>, String> {
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let namespace = params.namespace;
    let tags = params.tags;
    let document_id = documents
        .put_document(NamespaceDocumentInput {
            namespace: namespace.clone(),
            key: params.key,
            title: params.title,
            content: params.content,
            source_type: params.source_type,
            priority: params.priority,
            tags: tags.clone(),
            metadata: params.metadata,
            category: params.category,
            session_id: params.session_id,
            document_id: params.document_id,
            taint: crate::openhuman::memory::api::types::MemoryTaint::Internal,
        })
        .await
        .map_err(|error| error.to_string())?;
    // Chunking and graph extraction are driver-owned behind `put_document`.
    // The historical RPC response exposed the embedded engine's synchronous
    // extraction details, which a module boundary cannot observe. Preserve the
    // wire shape while reporting only the facts the host actually knows.
    let _driver_owned_config = params.config;
    let result = MemoryIngestionResult {
        document_id,
        namespace,
        model_name: "driver-managed".to_string(),
        extraction_mode: "driver-managed".to_string(),
        chunk_count: 0,
        entity_count: 0,
        relation_count: 0,
        preference_count: 0,
        decision_count: 0,
        tags,
        entities: Vec::new(),
        relations: Vec::new(),
    };
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
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let docs = documents
        .list_documents(params.as_ref().map(|value| value.namespace.as_str()))
        .await
        .map_err(|error| error.to_string())?;
    Ok(RpcOutcome::single_log(docs, "memory documents listed"))
}

/// Deletes a document from a namespace.
pub async fn doc_delete(params: DeleteDocParams) -> Result<RpcOutcome<serde_json::Value>, String> {
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let result = documents
        .delete_document(&params.namespace, &params.document_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(RpcOutcome::single_log(result, "memory document deleted"))
}

/// Clears all data within a namespace.
pub async fn clear_namespace(
    params: ClearNamespaceParams,
) -> Result<RpcOutcome<ClearNamespaceResult>, String> {
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    log::debug!("[memory] clear_namespace RPC invoked");
    documents
        .clear_namespace(&params.namespace)
        .await
        .map_err(|error| error.to_string())?;
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
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let result = documents
        .query_documents(
            &params.namespace,
            &params.query,
            params.limit.unwrap_or(10) as usize,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(RpcOutcome::single_log(
        result.context_text,
        "memory context queried",
    ))
}

/// Recalls contextual information from a namespace without a specific query.
pub async fn context_recall(
    params: RecallNamespaceParams,
) -> Result<RpcOutcome<Option<String>>, String> {
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let result = documents
        .recall_documents(&params.namespace, params.limit.unwrap_or(10) as usize)
        .await
        .map_err(|error| error.to_string())?;
    let context = (!result.context_text.is_empty()).then_some(result.context_text);
    Ok(RpcOutcome::single_log(context, "memory context recalled"))
}

// ---------------------------------------------------------------------------
// Envelope-style façade (`memory_*`)
// ---------------------------------------------------------------------------

/// Initialise the local-only (SQLite) memory subsystem for the current workspace.
///
/// `request.jwt_token` is accepted for backward compatibility but ignored — all
/// memory operations are local.  Remote/cloud sync is a future consideration.
///
/// "Initialise" now means **bind the memory driver for this workspace**, not
/// construct an in-process engine: this was `tinymemory_core::global::init`, a
/// second `MemoryClient` over the same SQLite file as the bound driver (#5560).
/// `active_memory_guard` is the cached, workspace-keyed resolution every other
/// handler in this file already takes, so calling it here warms exactly the
/// driver the next `memory_*` call will use.
///
/// One behaviour difference, stated rather than hidden: binding is infallible
/// by design — an inadmissible driver *falls back* and records why, where
/// `global::init` returned `Err`. So a driver that cannot bind is now reported
/// through `memory.provider_status` instead of failing this call, and only an
/// unresolvable workspace or a poisoned binding lock still errors. The driver
/// id is logged here so the distinction is not invisible at this handler
/// either.
pub async fn memory_init(
    request: MemoryInitRequest,
) -> Result<RpcOutcome<ApiEnvelope<MemoryInitResponse>>, String> {
    let _ = request.jwt_token; // accepted but unused — memory is local-only
    let workspace_dir = current_workspace_dir().await?;
    // Resolve (and thereby warm) the guarded driver for this workspace — the
    // same door every other handler in this file takes, so `memory_init`
    // initialises exactly what the next `memory_*` call will use.
    let guard = active_memory_guard().await?;
    log::debug!(
        "[memory:ops] memory_init: workspace={} driver={}",
        workspace_dir.display(),
        guard.driver_id(),
    );
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
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let raw = documents
        .list_documents(request.namespace.as_deref())
        .await
        .map_err(|error| error.to_string())?;
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

/// `openhuman.memory_namespace_summaries` result: per-namespace document
/// counts plus the grand total — the "did my stuff actually land" number.
#[derive(Debug, serde::Serialize)]
pub struct NamespaceSummariesResponse {
    pub namespaces: Vec<crate::openhuman::memory::api::types::NamespaceSummary>,
    pub total_documents: u64,
}

/// `openhuman.memory_namespace_summaries` — the mandatory `namespaces()`
/// surface, exposed: name, stored-document count and last-updated per
/// namespace. `list_namespaces` answers names alone, and a user verifying a
/// sync needs the count — a Gmail source that stored 1,120 documents should
/// be checkable as exactly that number (#5932 field finding).
pub async fn memory_namespace_summaries(
) -> Result<crate::rpc::RpcOutcome<NamespaceSummariesResponse>, String> {
    use crate::openhuman::memory::api::provider::MemoryCore;
    let guard = active_memory_guard().await?;
    let namespaces = guard
        .namespaces()
        .await
        .map_err(|error| error.to_string())?;
    let total_documents = namespaces.iter().map(|n| n.count as u64).sum();
    Ok(crate::rpc::RpcOutcome::new(
        NamespaceSummariesResponse {
            namespaces,
            total_documents,
        },
        vec![],
    ))
}

/// Lists all namespaces that contain memory documents.
pub async fn memory_list_namespaces(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListNamespacesResponse>>, String> {
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let namespaces = documents
        .list_namespaces()
        .await
        .map_err(|error| error.to_string())?;
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
    let guard = active_memory_guard().await?;
    let documents = guard
        .as_documents()
        .ok_or_else(|| "memory driver does not support the documents family".to_string())?;
    let raw = documents
        .delete_document(&request.namespace, &request.document_id)
        .await
        .map_err(|error| error.to_string())?;
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
        use tinymemory_api::provider::MemoryProvider;

        let guard = active_memory_guard().await?;
        let retrieval_limit = query_limit_for_request(guard.as_ref(), &request).await?;
        // `recall_namespace_scored` is the contract twin of
        // `query_namespace_context_data`: both resolve to the query-ranked
        // `query_namespace_hits` path, and the only thing the wrapper added was
        // a rendered `context_text` this handler never read — it builds its own
        // via `build_retrieval_context` / `format_llm_context_message`.
        //
        // `exclude_session_id: None` preserves the current behaviour exactly.
        // The self-echo exclusion belongs to an agent turn recalling mid-turn;
        // an RPC handler is not one, and passing a session here would start
        // silently hiding rows from a caller that used to see them.
        //
        // NOTE for whoever migrates the two sibling handlers below: this
        // mapping does **not** generalise. `memory.recall_context` and
        // `memory.recall_memories` take the *recency* path
        // (`recall_namespace_memories`), which is a different code path with no
        // bus twin — an empty query does not degrade to recency. See the gap
        // note in `memory::direct_engine_refs_tests`.
        let hits = guard
            .as_retrieval()
            .ok_or_else(|| "memory driver does not support the retrieval family".to_string())?
            .recall_namespace_scored(
                &request.namespace,
                &request.query,
                retrieval_limit as usize,
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        let mut context = NamespaceRetrievalContext {
            namespace: request.namespace.clone(),
            query: Some(request.query.clone()),
            context_text: String::new(),
            hits,
        };
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
    // The recency path, through the contract. `recall_namespace_recent` exists
    // for exactly this pair of handlers — `recall_namespace_scored("")` is NOT
    // a substitute, it ranks against nothing (see the member's doc). The
    // engine wrapper this used to call (`recall_namespace_context_data`) only
    // added a rendered `context_text` this handler never read; it builds its
    // own from the hits.
    let result = async {
        let guard = active_memory_guard().await?;
        guard
            .as_retrieval()
            .ok_or_else(|| "memory driver does not support the retrieval family".to_string())?
            .recall_namespace_recent(&request.namespace, request.resolved_limit() as usize)
            .await
            .map_err(|e| format!("memory.recall_context: {e}"))
    }
    .await;

    match result {
        Ok(hits) => {
            let retrieval_context = build_retrieval_context(&hits);
            let counts = memory_counts([
                ("num_entities", retrieval_context.entities.len()),
                ("num_relations", retrieval_context.relations.len()),
                ("num_chunks", retrieval_context.chunks.len()),
            ]);
            let llm_context_message = format_llm_context_message(None, &hits);
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
        let guard = active_memory_guard().await?;
        guard
            .as_retrieval()
            .ok_or_else(|| "memory driver does not support the retrieval family".to_string())?
            .recall_namespace_recent(&request.namespace, request.resolved_limit() as usize)
            .await
            .map_err(|e| format!("memory.recall_memories: {e}"))
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
#[path = "documents_tests.rs"]
mod tests;
