//! Controller schemas and handler dispatch for the `whatsapp_data` namespace.
//!
//! Agent-facing (read-only) RPC methods:
//!   - `openhuman.whatsapp_data_list_chats`
//!   - `openhuman.whatsapp_data_list_messages`
//!   - `openhuman.whatsapp_data_search_messages`
//!
//! Internal write path (NOT exposed to the agent controller registry):
//!   - `openhuman.whatsapp_data_ingest` — called by the Tauri scanner only
//!
//! Keeping ingest off the agent-facing registry prevents an agent from
//! mutating or poisoning the local WhatsApp store directly.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

/// Returns controller schemas advertised to the agent (read-only subset).
/// The ingest schema is intentionally excluded — it is an internal write path
/// called by the scanner, not something the agent should be able to invoke.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_chats"),
        schemas("list_messages"),
        schemas("search_messages"),
    ]
}

/// Returns registered controllers for the agent-facing dispatcher (read-only).
/// The ingest handler is registered separately via `all_internal_controllers()`
/// and wired by the scanner — not through the agent controller registry.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_chats"),
            handler: handle_list_chats,
        },
        RegisteredController {
            schema: schemas("list_messages"),
            handler: handle_list_messages,
        },
        RegisteredController {
            schema: schemas("search_messages"),
            handler: handle_search_messages,
        },
    ]
}

/// Returns the full controller set including the internal ingest handler.
/// Used by the core RPC dispatcher so the scanner can call
/// `openhuman.whatsapp_data_ingest` over JSON-RPC without exposing it to agents.
pub fn all_internal_controllers() -> Vec<RegisteredController> {
    let mut controllers = all_registered_controllers();
    controllers.insert(
        0,
        RegisteredController {
            schema: schemas("ingest"),
            handler: handle_ingest,
        },
    );
    controllers
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "ingest" => ControllerSchema {
            namespace: "whatsapp_data",
            function: "ingest",
            description: "Ingest a WhatsApp Web scanner snapshot (chats + messages). \
                          Called by the Tauri scanner after each CDP tick. Data is stored \
                          locally only — never transmitted externally.",
            inputs: vec![
                required_string("account_id", "WhatsApp account identifier (phone JID)."),
                FieldSchema {
                    name: "chats",
                    ty: TypeSchema::Json,
                    comment: "Map of chat JID → {name: string | null}.",
                    required: true,
                },
                FieldSchema {
                    name: "messages",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                    comment: "Array of message objects to upsert.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Ingest summary: chats_upserted, messages_upserted, messages_pruned.",
                required: true,
            }],
        },
        "list_chats" => ControllerSchema {
            namespace: "whatsapp_data",
            function: "list_chats",
            description: "List locally-stored WhatsApp chats, ordered by most recent message. \
                          When account_id is omitted, chats from all accounts are returned.",
            inputs: vec![
                optional_string("account_id", "Filter to a specific WhatsApp account."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum results (default 50).",
                    required: false,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Pagination offset (default 0).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "chats",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of WhatsAppChat objects.",
                required: true,
            }],
        },
        "list_messages" => ControllerSchema {
            namespace: "whatsapp_data",
            function: "list_messages",
            description: "List messages for a WhatsApp chat, ordered by timestamp ascending. \
                          When account_id is omitted, messages from all accounts are included.",
            inputs: vec![
                required_string("chat_id", "JID of the chat to retrieve messages for."),
                optional_string("account_id", "Filter to a specific WhatsApp account."),
                FieldSchema {
                    name: "since_ts",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Only return messages at or after this Unix timestamp (seconds).",
                    required: false,
                },
                FieldSchema {
                    name: "until_ts",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Only return messages at or before this Unix timestamp (seconds).",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum results (default 100).",
                    required: false,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Pagination offset (default 0).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "messages",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of WhatsAppMessage objects.",
                required: true,
            }],
        },
        "search_messages" => ControllerSchema {
            namespace: "whatsapp_data",
            function: "search_messages",
            description:
                "Search locally-stored WhatsApp message bodies. \
                          Case-insensitive substring match. \
                          When account_id / chat_id are omitted, all accounts / chats are searched.",
            inputs: vec![
                required_string("query", "Search query matched against message bodies."),
                optional_string("chat_id", "Restrict search to a specific chat JID."),
                optional_string(
                    "account_id",
                    "Restrict search to a specific WhatsApp account.",
                ),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum results (default 20).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "messages",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of WhatsAppMessage objects matching the query.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "whatsapp_data",
            function: "unknown",
            description: "Unknown whatsapp_data controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

fn handle_ingest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = deserialize_params(params)?;
        to_json(crate::openhuman::whatsapp_data::rpc::whatsapp_data_ingest(req).await?)
    })
}

fn handle_list_chats(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = deserialize_params(params)?;
        to_json(crate::openhuman::whatsapp_data::rpc::whatsapp_data_list_chats(req).await?)
    })
}

fn handle_list_messages(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = deserialize_params(params)?;
        to_json(crate::openhuman::whatsapp_data::rpc::whatsapp_data_list_messages(req).await?)
    })
}

fn handle_search_messages(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = deserialize_params(params)?;
        to_json(crate::openhuman::whatsapp_data::rpc::whatsapp_data_search_messages(req).await?)
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}
