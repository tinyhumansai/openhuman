//! Schemas and handlers for document, namespace, recall, and clear-namespace
//! RPC methods.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc::{
    self, ClearNamespaceParams, DeleteDocParams, IngestDocParams, NamespaceOnlyParams,
    PutDocParams, QueryNamespaceParams, RecallNamespaceParams,
};
use crate::openhuman::memory::{
    DeleteDocumentRequest, EmptyRequest, ListDocumentsRequest, MemoryInitRequest,
    QueryNamespaceRequest, RecallContextRequest, RecallMemoriesRequest,
};

use super::{parse_params, to_json};

pub(super) const FUNCTIONS: &[&str] = &[
    "init",
    "list_documents",
    "list_namespaces",
    "delete_document",
    "query_namespace",
    "recall_context",
    "recall_memories",
    "namespace_list",
    "doc_put",
    "doc_ingest",
    "doc_list",
    "doc_delete",
    "context_query",
    "context_recall",
    "clear_namespace",
];

pub(super) fn controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("init").unwrap(),
            handler: handle_init,
        },
        RegisteredController {
            schema: schema("list_documents").unwrap(),
            handler: handle_list_documents,
        },
        RegisteredController {
            schema: schema("list_namespaces").unwrap(),
            handler: handle_list_namespaces,
        },
        RegisteredController {
            schema: schema("delete_document").unwrap(),
            handler: handle_delete_document,
        },
        RegisteredController {
            schema: schema("query_namespace").unwrap(),
            handler: handle_query_namespace,
        },
        RegisteredController {
            schema: schema("recall_context").unwrap(),
            handler: handle_recall_context,
        },
        RegisteredController {
            schema: schema("recall_memories").unwrap(),
            handler: handle_recall_memories,
        },
        RegisteredController {
            schema: schema("namespace_list").unwrap(),
            handler: handle_namespace_list,
        },
        RegisteredController {
            schema: schema("doc_put").unwrap(),
            handler: handle_doc_put,
        },
        RegisteredController {
            schema: schema("doc_ingest").unwrap(),
            handler: handle_doc_ingest,
        },
        RegisteredController {
            schema: schema("doc_list").unwrap(),
            handler: handle_doc_list,
        },
        RegisteredController {
            schema: schema("doc_delete").unwrap(),
            handler: handle_doc_delete,
        },
        RegisteredController {
            schema: schema("context_query").unwrap(),
            handler: handle_context_query,
        },
        RegisteredController {
            schema: schema("context_recall").unwrap(),
            handler: handle_context_recall,
        },
        RegisteredController {
            schema: schema("clear_namespace").unwrap(),
            handler: handle_clear_namespace,
        },
    ]
}

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    Some(match function {
        "init" => ControllerSchema {
            namespace: "memory",
            function: "init",
            description: "Initialise the local-only (SQLite) memory subsystem for the current workspace. The jwt_token parameter is accepted for backward compatibility but ignored — memory is entirely local.",
            inputs: vec![FieldSchema {
                name: "jwt_token",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Accepted for backward compatibility but ignored — memory is local-only. Remote sync is a future consideration.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with initialisation status, workspace and memory paths.",
                required: true,
            }],
        },
        "list_documents" => ControllerSchema {
            namespace: "memory",
            function: "list_documents",
            description: "List documents stored in memory, optionally filtered by namespace.",
            inputs: vec![FieldSchema {
                name: "namespace",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Namespace to filter documents by.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with documents array and count.",
                required: true,
            }],
        },
        "list_namespaces" => ControllerSchema {
            namespace: "memory",
            function: "list_namespaces",
            description: "List all namespaces that contain memory documents.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with namespaces array and count.",
                required: true,
            }],
        },
        "delete_document" => ControllerSchema {
            namespace: "memory",
            function: "delete_document",
            description: "Delete a specific document from a namespace.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "Namespace containing the document.",
                    required: true,
                },
                FieldSchema {
                    name: "document_id",
                    ty: TypeSchema::String,
                    comment: "Identifier of the document to delete.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with deletion status.",
                required: true,
            }],
        },
        "query_namespace" => ControllerSchema {
            namespace: "memory",
            function: "query_namespace",
            description: "Semantic query against a namespace with optional reference data.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "Namespace to query.",
                    required: true,
                },
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Natural-language query string.",
                    required: true,
                },
                FieldSchema {
                    name: "include_references",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Whether to include entity/relation context in the response.",
                    required: false,
                },
                FieldSchema {
                    name: "document_ids",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Restrict results to these document IDs.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of results to return.",
                    required: false,
                },
                FieldSchema {
                    name: "max_chunks",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of chunks to return (alias for limit).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with retrieval context and LLM context message.",
                required: true,
            }],
        },
        "recall_context" => ControllerSchema {
            namespace: "memory",
            function: "recall_context",
            description: "Recall contextual data from a namespace without a specific query.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "Namespace to recall from.",
                    required: true,
                },
                FieldSchema {
                    name: "include_references",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Whether to include entity/relation context.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of results.",
                    required: false,
                },
                FieldSchema {
                    name: "max_chunks",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of chunks (alias for limit).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with retrieval context and LLM context message.",
                required: true,
            }],
        },
        "recall_memories" => ControllerSchema {
            namespace: "memory",
            function: "recall_memories",
            description: "Recall memory items from a namespace with optional retention filtering.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "Namespace to recall memories from.",
                    required: true,
                },
                FieldSchema {
                    name: "min_retention",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Minimum retention score (forward-compat, currently ignored).",
                    required: false,
                },
                FieldSchema {
                    name: "as_of",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Temporal recall timestamp (forward-compat, currently ignored).",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of results.",
                    required: false,
                },
                FieldSchema {
                    name: "max_chunks",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of chunks (alias for limit).",
                    required: false,
                },
                FieldSchema {
                    name: "top_k",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Top-k override (alias for limit).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with recalled memory items.",
                required: true,
            }],
        },
        "namespace_list" => ControllerSchema {
            namespace: "memory",
            function: "namespace_list",
            description: "List all namespaces in the unified memory store.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "namespaces",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Namespace names.",
                required: true,
            }],
        },
        "doc_put" => ControllerSchema {
            namespace: "memory",
            function: "doc_put",
            description: "Upsert a document into a namespace.",
            inputs: vec![
                FieldSchema { name: "namespace", ty: TypeSchema::String, comment: "Target namespace.", required: true },
                FieldSchema { name: "key", ty: TypeSchema::String, comment: "Document key for upsert deduplication.", required: true },
                FieldSchema { name: "title", ty: TypeSchema::String, comment: "Human-readable title.", required: true },
                FieldSchema { name: "content", ty: TypeSchema::String, comment: "Document body content.", required: true },
                FieldSchema { name: "source_type", ty: TypeSchema::String, comment: "Source type label (default: \"doc\").", required: false },
                FieldSchema { name: "priority", ty: TypeSchema::String, comment: "Priority level (default: \"medium\").", required: false },
                FieldSchema { name: "tags", ty: TypeSchema::Array(Box::new(TypeSchema::String)), comment: "Tags for categorisation (default: []).", required: false },
                FieldSchema { name: "metadata", ty: TypeSchema::Json, comment: "Arbitrary metadata (default: {}).", required: false },
                FieldSchema { name: "category", ty: TypeSchema::String, comment: "Memory category (default: \"core\").", required: false },
                FieldSchema { name: "session_id", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Optional session ID for provenance tracking.", required: false },
                FieldSchema { name: "document_id", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Optional explicit document ID; generated if omitted.", required: false },
            ],
            outputs: vec![FieldSchema { name: "document_id", ty: TypeSchema::String, comment: "ID of the upserted document.", required: true }],
        },
        "doc_ingest" => ControllerSchema {
            namespace: "memory",
            function: "doc_ingest",
            description: "Ingest a document with entity/relation extraction and chunk embedding.",
            inputs: vec![
                FieldSchema { name: "namespace", ty: TypeSchema::String, comment: "Target namespace.", required: true },
                FieldSchema { name: "key", ty: TypeSchema::String, comment: "Document key.", required: true },
                FieldSchema { name: "title", ty: TypeSchema::String, comment: "Human-readable title.", required: true },
                FieldSchema { name: "content", ty: TypeSchema::String, comment: "Document body content.", required: true },
                FieldSchema { name: "source_type", ty: TypeSchema::String, comment: "Source type label (default: \"doc\").", required: false },
                FieldSchema { name: "priority", ty: TypeSchema::String, comment: "Priority level (default: \"medium\").", required: false },
                FieldSchema { name: "tags", ty: TypeSchema::Array(Box::new(TypeSchema::String)), comment: "Tags for categorisation (default: []).", required: false },
                FieldSchema { name: "metadata", ty: TypeSchema::Json, comment: "Arbitrary metadata (default: {}).", required: false },
                FieldSchema { name: "category", ty: TypeSchema::String, comment: "Memory category (default: \"core\").", required: false },
                FieldSchema { name: "session_id", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Optional session ID.", required: false },
                FieldSchema { name: "document_id", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Optional explicit document ID.", required: false },
                FieldSchema { name: "config", ty: TypeSchema::Option(Box::new(TypeSchema::Json)), comment: "Optional ingestion configuration overrides.", required: false },
            ],
            outputs: vec![FieldSchema { name: "result", ty: TypeSchema::Json, comment: "Ingestion result with entity, relation and chunk counts.", required: true }],
        },
        "doc_list" => ControllerSchema {
            namespace: "memory",
            function: "doc_list",
            description: "List documents in the unified memory store, optionally by namespace.",
            inputs: vec![FieldSchema {
                name: "namespace",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional namespace filter.",
                required: false,
            }],
            outputs: vec![FieldSchema { name: "result", ty: TypeSchema::Json, comment: "Document listing.", required: true }],
        },
        "doc_delete" => ControllerSchema {
            namespace: "memory",
            function: "doc_delete",
            description: "Delete a document from the unified memory store.",
            inputs: vec![
                FieldSchema { name: "namespace", ty: TypeSchema::String, comment: "Namespace containing the document.", required: true },
                FieldSchema { name: "document_id", ty: TypeSchema::String, comment: "Document identifier to delete.", required: true },
            ],
            outputs: vec![FieldSchema { name: "result", ty: TypeSchema::Json, comment: "Deletion result.", required: true }],
        },
        "context_query" => ControllerSchema {
            namespace: "memory",
            function: "context_query",
            description: "Query a namespace for contextual information.",
            inputs: vec![
                FieldSchema { name: "namespace", ty: TypeSchema::String, comment: "Namespace to query.", required: true },
                FieldSchema { name: "query", ty: TypeSchema::String, comment: "Natural-language query string.", required: true },
                FieldSchema { name: "limit", ty: TypeSchema::Option(Box::new(TypeSchema::U64)), comment: "Maximum number of results.", required: false },
            ],
            outputs: vec![FieldSchema { name: "result", ty: TypeSchema::String, comment: "Contextual query result string.", required: true }],
        },
        "context_recall" => ControllerSchema {
            namespace: "memory",
            function: "context_recall",
            description: "Recall context from a namespace.",
            inputs: vec![
                FieldSchema { name: "namespace", ty: TypeSchema::String, comment: "Namespace to recall from.", required: true },
                FieldSchema { name: "limit", ty: TypeSchema::Option(Box::new(TypeSchema::U64)), comment: "Maximum number of results.", required: false },
            ],
            outputs: vec![FieldSchema { name: "result", ty: TypeSchema::Json, comment: "Recalled context (may be null if empty).", required: true }],
        },
        "clear_namespace" => ControllerSchema {
            namespace: "memory",
            function: "clear_namespace",
            description: "Delete all documents, vector chunks, KV entries, and graph relations for a namespace.",
            inputs: vec![FieldSchema {
                name: "namespace",
                ty: TypeSchema::String,
                comment: "Namespace to clear completely.",
                required: true,
            }],
            outputs: vec![
                FieldSchema { name: "cleared", ty: TypeSchema::Bool, comment: "True when the namespace was cleared.", required: true },
                FieldSchema { name: "namespace", ty: TypeSchema::String, comment: "The namespace that was cleared.", required: true },
            ],
        },
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn handle_init(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<MemoryInitRequest>(params)?;
        to_json(rpc::memory_init(payload).await?)
    })
}

fn handle_list_documents(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ListDocumentsRequest>(params)?;
        to_json(rpc::memory_list_documents(payload).await?)
    })
}

fn handle_list_namespaces(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::memory_list_namespaces(EmptyRequest {}).await?) })
}

fn handle_delete_document(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<DeleteDocumentRequest>(params)?;
        to_json(rpc::memory_delete_document(payload).await?)
    })
}

fn handle_query_namespace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<QueryNamespaceRequest>(params)?;
        to_json(rpc::memory_query_namespace(payload).await?)
    })
}

fn handle_recall_context(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<RecallContextRequest>(params)?;
        to_json(rpc::memory_recall_context(payload).await?)
    })
}

fn handle_recall_memories(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<RecallMemoriesRequest>(params)?;
        to_json(rpc::memory_recall_memories(payload).await?)
    })
}

fn handle_namespace_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::namespace_list().await?) })
}

fn handle_doc_put(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<PutDocParams>(params)?;
        to_json(rpc::doc_put(payload).await?)
    })
}

fn handle_doc_ingest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<IngestDocParams>(params)?;
        to_json(rpc::doc_ingest(payload).await?)
    })
}

#[derive(serde::Deserialize)]
struct DocListParams {
    #[serde(default)]
    namespace: Option<String>,
}

fn handle_doc_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // Reject invalid `namespace` types (e.g. `123`, `["x"]`) instead of
        // silently coercing to `None` and returning an unscoped document list.
        let parsed: DocListParams = parse_params(params)?;
        let namespace = parsed
            .namespace
            .map(|namespace| NamespaceOnlyParams { namespace });
        to_json(rpc::doc_list(namespace).await?)
    })
}

fn handle_doc_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<DeleteDocParams>(params)?;
        to_json(rpc::doc_delete(payload).await?)
    })
}

fn handle_context_query(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<QueryNamespaceParams>(params)?;
        to_json(rpc::context_query(payload).await?)
    })
}

fn handle_context_recall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<RecallNamespaceParams>(params)?;
        to_json(rpc::context_recall(payload).await?)
    })
}

fn handle_clear_namespace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ClearNamespaceParams>(params)?;
        to_json(rpc::clear_namespace(payload).await?)
    })
}
