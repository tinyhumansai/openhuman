//! RPC schemas and controller registration for the memory system.
//!
//! This module defines the metadata (schemas) for all memory-related RPC functions
//! and registers their corresponding handlers. It serves as the bridge between
//! the RPC system and the underlying memory operations.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc::{
    self, ClearNamespaceParams, DeleteDocParams, GraphQueryParams, GraphUpsertParams,
    IngestDocParams, KvGetDeleteParams, KvSetParams, NamespaceOnlyParams, PutDocParams,
    QueryNamespaceParams, RecallNamespaceParams,
};
use crate::openhuman::memory::{
    DeleteDocumentRequest, EmptyRequest, ListDocumentsRequest, ListMemoryFilesRequest,
    MemoryInitRequest, QueryNamespaceRequest, ReadMemoryFileRequest, RecallContextRequest,
    RecallMemoriesRequest, WriteMemoryFileRequest,
};
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Returns all controller schemas for the memory system.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("init"),
        schemas("list_documents"),
        schemas("list_namespaces"),
        schemas("delete_document"),
        schemas("query_namespace"),
        schemas("recall_context"),
        schemas("recall_memories"),
        schemas("list_files"),
        schemas("read_file"),
        schemas("write_file"),
        schemas("namespace_list"),
        schemas("doc_put"),
        schemas("doc_ingest"),
        schemas("doc_list"),
        schemas("doc_delete"),
        schemas("context_query"),
        schemas("context_recall"),
        schemas("kv_set"),
        schemas("kv_get"),
        schemas("kv_delete"),
        schemas("kv_list_namespace"),
        schemas("graph_upsert"),
        schemas("graph_query"),
        schemas("clear_namespace"),
    ]
}

/// Returns all registered controllers for the memory system, mapping schemas to handlers.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("init"),
            handler: handle_init,
        },
        RegisteredController {
            schema: schemas("list_documents"),
            handler: handle_list_documents,
        },
        RegisteredController {
            schema: schemas("list_namespaces"),
            handler: handle_list_namespaces,
        },
        RegisteredController {
            schema: schemas("delete_document"),
            handler: handle_delete_document,
        },
        RegisteredController {
            schema: schemas("query_namespace"),
            handler: handle_query_namespace,
        },
        RegisteredController {
            schema: schemas("recall_context"),
            handler: handle_recall_context,
        },
        RegisteredController {
            schema: schemas("recall_memories"),
            handler: handle_recall_memories,
        },
        RegisteredController {
            schema: schemas("list_files"),
            handler: handle_list_files,
        },
        RegisteredController {
            schema: schemas("read_file"),
            handler: handle_read_file,
        },
        RegisteredController {
            schema: schemas("write_file"),
            handler: handle_write_file,
        },
        RegisteredController {
            schema: schemas("namespace_list"),
            handler: handle_namespace_list,
        },
        RegisteredController {
            schema: schemas("doc_put"),
            handler: handle_doc_put,
        },
        RegisteredController {
            schema: schemas("doc_ingest"),
            handler: handle_doc_ingest,
        },
        RegisteredController {
            schema: schemas("doc_list"),
            handler: handle_doc_list,
        },
        RegisteredController {
            schema: schemas("doc_delete"),
            handler: handle_doc_delete,
        },
        RegisteredController {
            schema: schemas("context_query"),
            handler: handle_context_query,
        },
        RegisteredController {
            schema: schemas("context_recall"),
            handler: handle_context_recall,
        },
        RegisteredController {
            schema: schemas("kv_set"),
            handler: handle_kv_set,
        },
        RegisteredController {
            schema: schemas("kv_get"),
            handler: handle_kv_get,
        },
        RegisteredController {
            schema: schemas("kv_delete"),
            handler: handle_kv_delete,
        },
        RegisteredController {
            schema: schemas("kv_list_namespace"),
            handler: handle_kv_list_namespace,
        },
        RegisteredController {
            schema: schemas("graph_upsert"),
            handler: handle_graph_upsert,
        },
        RegisteredController {
            schema: schemas("graph_query"),
            handler: handle_graph_query,
        },
        RegisteredController {
            schema: schemas("clear_namespace"),
            handler: handle_clear_namespace,
        },
    ]
}

// ---------------------------------------------------------------------------
// Schema definitions
// ---------------------------------------------------------------------------

/// Defines the schema for a specific memory controller function.
pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        // ----- legacy envelope-style methods -----
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

        // ----- file-based memory methods -----
        "list_files" => ControllerSchema {
            namespace: "memory",
            function: "list_files",
            description: "List files in a memory directory.",
            inputs: vec![FieldSchema {
                name: "relative_dir",
                ty: TypeSchema::String,
                comment: "Relative directory path under the workspace (default: \"memory\").",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with file listing.",
                required: true,
            }],
        },
        "read_file" => ControllerSchema {
            namespace: "memory",
            function: "read_file",
            description: "Read the contents of a memory file.",
            inputs: vec![FieldSchema {
                name: "relative_path",
                ty: TypeSchema::String,
                comment: "Relative path to the file under the workspace memory directory.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with file content.",
                required: true,
            }],
        },
        "write_file" => ControllerSchema {
            namespace: "memory",
            function: "write_file",
            description: "Write content to a memory file.",
            inputs: vec![
                FieldSchema {
                    name: "relative_path",
                    ty: TypeSchema::String,
                    comment: "Relative path to the file under the workspace memory directory.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Content to write to the file.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with write confirmation and bytes written.",
                required: true,
            }],
        },
        // ----- unified memory API methods -----
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
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "Target namespace.",
                    required: true,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Document key for upsert deduplication.",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "Human-readable title.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Document body content.",
                    required: true,
                },
                FieldSchema {
                    name: "source_type",
                    ty: TypeSchema::String,
                    comment: "Source type label (default: \"doc\").",
                    required: false,
                },
                FieldSchema {
                    name: "priority",
                    ty: TypeSchema::String,
                    comment: "Priority level (default: \"medium\").",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Tags for categorisation (default: []).",
                    required: false,
                },
                FieldSchema {
                    name: "metadata",
                    ty: TypeSchema::Json,
                    comment: "Arbitrary metadata (default: {}).",
                    required: false,
                },
                FieldSchema {
                    name: "category",
                    ty: TypeSchema::String,
                    comment: "Memory category (default: \"core\").",
                    required: false,
                },
                FieldSchema {
                    name: "session_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional session ID for provenance tracking.",
                    required: false,
                },
                FieldSchema {
                    name: "document_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional explicit document ID; generated if omitted.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "document_id",
                ty: TypeSchema::String,
                comment: "ID of the upserted document.",
                required: true,
            }],
        },
        "doc_ingest" => ControllerSchema {
            namespace: "memory",
            function: "doc_ingest",
            description: "Ingest a document with entity/relation extraction and chunk embedding.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "Target namespace.",
                    required: true,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Document key.",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "Human-readable title.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Document body content.",
                    required: true,
                },
                FieldSchema {
                    name: "source_type",
                    ty: TypeSchema::String,
                    comment: "Source type label (default: \"doc\").",
                    required: false,
                },
                FieldSchema {
                    name: "priority",
                    ty: TypeSchema::String,
                    comment: "Priority level (default: \"medium\").",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Tags for categorisation (default: []).",
                    required: false,
                },
                FieldSchema {
                    name: "metadata",
                    ty: TypeSchema::Json,
                    comment: "Arbitrary metadata (default: {}).",
                    required: false,
                },
                FieldSchema {
                    name: "category",
                    ty: TypeSchema::String,
                    comment: "Memory category (default: \"core\").",
                    required: false,
                },
                FieldSchema {
                    name: "session_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional session ID.",
                    required: false,
                },
                FieldSchema {
                    name: "document_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional explicit document ID.",
                    required: false,
                },
                FieldSchema {
                    name: "config",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional ingestion configuration overrides.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Ingestion result with entity, relation and chunk counts.",
                required: true,
            }],
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
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Document listing.",
                required: true,
            }],
        },
        "doc_delete" => ControllerSchema {
            namespace: "memory",
            function: "doc_delete",
            description: "Delete a document from the unified memory store.",
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
                    comment: "Document identifier to delete.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Deletion result.",
                required: true,
            }],
        },
        "context_query" => ControllerSchema {
            namespace: "memory",
            function: "context_query",
            description: "Query a namespace for contextual information.",
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
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of results.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::String,
                comment: "Contextual query result string.",
                required: true,
            }],
        },
        "context_recall" => ControllerSchema {
            namespace: "memory",
            function: "context_recall",
            description: "Recall context from a namespace.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "Namespace to recall from.",
                    required: true,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of results.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Recalled context (may be null if empty).",
                required: true,
            }],
        },

        // ----- key-value methods -----
        "kv_set" => ControllerSchema {
            namespace: "memory",
            function: "kv_set",
            description: "Set a key-value pair in the memory store.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Key to set.",
                    required: true,
                },
                FieldSchema {
                    name: "value",
                    ty: TypeSchema::Json,
                    comment: "JSON value to store.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when the value was stored.",
                required: true,
            }],
        },
        "kv_get" => ControllerSchema {
            namespace: "memory",
            function: "kv_get",
            description: "Get a value by key from the memory store.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Key to retrieve.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Stored value or null if not found.",
                required: true,
            }],
        },
        "kv_delete" => ControllerSchema {
            namespace: "memory",
            function: "kv_delete",
            description: "Delete a key-value pair from the memory store.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Key to delete.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when the key was deleted.",
                required: true,
            }],
        },
        "kv_list_namespace" => ControllerSchema {
            namespace: "memory",
            function: "kv_list_namespace",
            description: "List all key-value entries in a namespace.",
            inputs: vec![FieldSchema {
                name: "namespace",
                ty: TypeSchema::String,
                comment: "Namespace to list.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of key-value entries.",
                required: true,
            }],
        },

        // ----- graph methods -----
        "graph_upsert" => ControllerSchema {
            namespace: "memory",
            function: "graph_upsert",
            description: "Upsert a relation triple in the knowledge graph.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "subject",
                    ty: TypeSchema::String,
                    comment: "Subject entity of the relation.",
                    required: true,
                },
                FieldSchema {
                    name: "predicate",
                    ty: TypeSchema::String,
                    comment: "Relation predicate.",
                    required: true,
                },
                FieldSchema {
                    name: "object",
                    ty: TypeSchema::String,
                    comment: "Object entity of the relation.",
                    required: true,
                },
                FieldSchema {
                    name: "attrs",
                    ty: TypeSchema::Json,
                    comment: "Extra attributes on the relation (default: {}).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when the relation was upserted.",
                required: true,
            }],
        },
        "graph_query" => ControllerSchema {
            namespace: "memory",
            function: "graph_query",
            description: "Query relations from the knowledge graph.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "subject",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Filter by subject entity.",
                    required: false,
                },
                FieldSchema {
                    name: "predicate",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Filter by relation predicate.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of matching relation records.",
                required: true,
            }],
        },

        // ----- bulk operations -----
        "clear_namespace" => ControllerSchema {
            namespace: "memory",
            function: "clear_namespace",
            description:
                "Delete all documents, vector chunks, KV entries, and graph relations for a namespace.",
            inputs: vec![FieldSchema {
                name: "namespace",
                ty: TypeSchema::String,
                comment: "Namespace to clear completely.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "cleared",
                    ty: TypeSchema::Bool,
                    comment: "True when the namespace was cleared.",
                    required: true,
                },
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::String,
                    comment: "The namespace that was cleared.",
                    required: true,
                },
            ],
        },

        // ----- fallback -----
        _other => ControllerSchema {
            namespace: "memory",
            function: "unknown",
            description: "Unknown memory controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
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

fn handle_list_files(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let relative_dir = params
            .get("relative_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("memory")
            .to_string();
        let payload = ListMemoryFilesRequest { relative_dir };
        to_json(rpc::ai_list_memory_files(payload).await?)
    })
}

fn handle_read_file(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ReadMemoryFileRequest>(params)?;
        to_json(rpc::ai_read_memory_file(payload).await?)
    })
}

fn handle_write_file(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<WriteMemoryFileRequest>(params)?;
        to_json(rpc::ai_write_memory_file(payload).await?)
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

fn handle_doc_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let namespace =
            params
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(|ns| NamespaceOnlyParams {
                    namespace: ns.to_string(),
                });
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

fn handle_kv_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<KvSetParams>(params)?;
        to_json(rpc::kv_set(payload).await?)
    })
}

fn handle_kv_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<KvGetDeleteParams>(params)?;
        to_json(rpc::kv_get(payload).await?)
    })
}

fn handle_kv_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<KvGetDeleteParams>(params)?;
        to_json(rpc::kv_delete(payload).await?)
    })
}

fn handle_kv_list_namespace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<NamespaceOnlyParams>(params)?;
        to_json(rpc::kv_list_namespace(payload).await?)
    })
}

fn handle_graph_upsert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<GraphUpsertParams>(params)?;
        to_json(rpc::graph_upsert(payload).await?)
    })
}

fn handle_graph_query(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<GraphQueryParams>(params)?;
        to_json(rpc::graph_query(payload).await?)
    })
}

fn handle_clear_namespace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ClearNamespaceParams>(params)?;
        to_json(rpc::clear_namespace(payload).await?)
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALL_FUNCTIONS: &[&str] = &[
        "init",
        "list_documents",
        "list_namespaces",
        "delete_document",
        "query_namespace",
        "recall_context",
        "recall_memories",
        "list_files",
        "read_file",
        "write_file",
        "namespace_list",
        "doc_put",
        "doc_ingest",
        "doc_list",
        "doc_delete",
        "context_query",
        "context_recall",
        "kv_set",
        "kv_get",
        "kv_delete",
        "kv_list_namespace",
        "graph_upsert",
        "graph_query",
        "clear_namespace",
    ];

    #[test]
    fn all_controller_schemas_has_entry_per_supported_function() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(names.len(), ALL_FUNCTIONS.len());
        for expected in ALL_FUNCTIONS {
            assert!(names.contains(expected), "missing schema for {expected}");
        }
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        let controllers = all_registered_controllers();
        assert_eq!(controllers.len(), ALL_FUNCTIONS.len());
        let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
        for expected in ALL_FUNCTIONS {
            assert!(names.contains(expected), "missing handler for {expected}");
        }
    }

    #[test]
    fn every_schema_uses_memory_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(
                s.namespace, "memory",
                "schema {} must use the memory namespace",
                s.function
            );
        }
    }

    #[test]
    fn every_schema_has_a_non_empty_description() {
        for s in all_controller_schemas() {
            assert!(
                !s.description.is_empty(),
                "schema {} has empty description",
                s.function
            );
        }
    }

    #[test]
    fn schemas_unknown_function_returns_unknown_placeholder() {
        let s = schemas("not-a-real-function");
        assert_eq!(s.namespace, "memory");
        assert_eq!(s.function, "unknown");
    }

    // ── parse_params helper ──────────────────────────────────────

    #[test]
    fn parse_params_deserializes_simple_struct() {
        #[derive(serde::Deserialize, Debug)]
        struct Simple {
            name: String,
            count: u32,
        }
        let mut m = Map::new();
        m.insert("name".into(), json!("hi"));
        m.insert("count".into(), json!(7));
        let out: Simple = parse_params(m).unwrap();
        assert_eq!(out.name, "hi");
        assert_eq!(out.count, 7);
    }

    #[test]
    fn parse_params_surfaces_deserialization_errors_with_context() {
        #[derive(serde::Deserialize, Debug)]
        struct Strict {
            #[allow(dead_code)]
            count: u32,
        }
        let mut m = Map::new();
        m.insert("count".into(), json!("not-a-number"));
        let err = parse_params::<Strict>(m).unwrap_err();
        assert!(err.contains("invalid params"));
    }
}
