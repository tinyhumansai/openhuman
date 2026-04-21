//! Controller schemas for the memory tree (Phase 1 / #707).
//!
//! Registered JSON-RPC methods:
//! - `openhuman.memory_tree_ingest`      — unified ingest (source_kind + JSON payload)
//! - `openhuman.memory_tree_list_chunks`
//! - `openhuman.memory_tree_get_chunk`
//!
//! Handlers delegate to [`super::rpc`].

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::rpc as tree_rpc;
use crate::rpc::RpcOutcome;

const NAMESPACE: &str = "memory_tree";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("ingest"),
        schemas("list_chunks"),
        schemas("get_chunk"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("ingest"),
            handler: handle_ingest,
        },
        RegisteredController {
            schema: schemas("list_chunks"),
            handler: handle_list_chunks,
        },
        RegisteredController {
            schema: schemas("get_chunk"),
            handler: handle_get_chunk,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "ingest" => ControllerSchema {
            namespace: NAMESPACE,
            function: "ingest",
            description: "Ingest a source into canonical chunks. \
                 Dispatches on `source_kind`; `payload` shape depends on the kind \
                 (chat → ChatBatch, email → EmailThread, document → DocumentInput).",
            inputs: vec![
                FieldSchema {
                    name: "source_kind",
                    ty: TypeSchema::Enum {
                        variants: vec!["chat", "email", "document"],
                    },
                    comment: "Which source kind the payload represents.",
                    required: true,
                },
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Stable logical source id (channel, thread, document id).",
                    required: true,
                },
                FieldSchema {
                    name: "owner",
                    ty: TypeSchema::String,
                    comment: "Optional account / user this content belongs to.",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tags or labels carried through.",
                    required: false,
                },
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Json,
                    comment: "Adapter-specific payload. \
                         chat: {platform, channel_label, messages[]}. \
                         email: {provider, thread_subject, messages[]}. \
                         document: {provider, title, body, modified_at, source_ref}.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Logical source id the ingest was scoped to.",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_written",
                    ty: TypeSchema::U64,
                    comment: "Number of chunks persisted (including idempotent rewrites).",
                    required: true,
                },
                FieldSchema {
                    name: "chunk_ids",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "IDs of all chunks produced.",
                    required: true,
                },
            ],
        },
        "list_chunks" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_chunks",
            description:
                "List stored chunks, newest first, optionally filtered by source / owner / time.",
            inputs: vec![
                FieldSchema {
                    name: "source_kind",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["chat", "email", "document"],
                    })),
                    comment: "Restrict to a single source kind.",
                    required: false,
                },
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to a single logical source (channel/thread/doc id).",
                    required: false,
                },
                FieldSchema {
                    name: "owner",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to a single owner/account.",
                    required: false,
                },
                FieldSchema {
                    name: "since_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Inclusive lower bound on chunk timestamp (ms since epoch).",
                    required: false,
                },
                FieldSchema {
                    name: "until_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Inclusive upper bound on chunk timestamp (ms since epoch).",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum rows to return (defaults to 100).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "chunks",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Chunk"))),
                comment: "Matching chunks ordered by timestamp DESC.",
                required: true,
            }],
        },
        "get_chunk" => ControllerSchema {
            namespace: NAMESPACE,
            function: "get_chunk",
            description: "Fetch a single chunk by its deterministic id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Chunk id (32 hex chars).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "chunk",
                ty: TypeSchema::Option(Box::new(TypeSchema::Ref("Chunk"))),
                comment: "The chunk if found, otherwise null.",
                required: false,
            }],
        },
        _ => ControllerSchema {
            namespace: NAMESPACE,
            function: "unknown",
            description: "Unknown memory_tree controller function.",
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

fn handle_ingest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<tree_rpc::IngestRequest>(Value::Object(params))?;
        to_json(tree_rpc::ingest_rpc(&config, req).await?)
    })
}

fn handle_list_chunks(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<tree_rpc::ListChunksRequest>(Value::Object(params))?;
        to_json(tree_rpc::list_chunks_rpc(&config, req).await?)
    })
}

fn handle_get_chunk(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<tree_rpc::GetChunkRequest>(Value::Object(params))?;
        to_json(tree_rpc::get_chunk_rpc(&config, req).await?)
    })
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
