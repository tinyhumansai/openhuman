//! RPC schemas and controller registration for conversation threads.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessagesRequest, DeleteConversationThreadRequest,
    EmptyRequest, UpdateConversationMessageRequest, UpsertConversationThreadRequest,
};

use super::ops;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("upsert"),
        schemas("create_new"),
        schemas("messages_list"),
        schemas("message_append"),
        schemas("message_update"),
        schemas("delete"),
        schemas("purge"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("upsert"),
            handler: handle_upsert,
        },
        RegisteredController {
            schema: schemas("create_new"),
            handler: handle_create_new,
        },
        RegisteredController {
            schema: schemas("messages_list"),
            handler: handle_messages_list,
        },
        RegisteredController {
            schema: schemas("message_append"),
            handler: handle_message_append,
        },
        RegisteredController {
            schema: schemas("message_update"),
            handler: handle_message_update,
        },
        RegisteredController {
            schema: schemas("delete"),
            handler: handle_delete,
        },
        RegisteredController {
            schema: schemas("purge"),
            handler: handle_purge,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "threads",
            function: "list",
            description: "List conversation threads.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with thread summaries and count.",
                required: true,
            }],
        },
        "upsert" => ControllerSchema {
            namespace: "threads",
            function: "upsert",
            description: "Create or refresh a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Stable thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "Human-readable thread title.",
                    required: true,
                },
                FieldSchema {
                    name: "created_at",
                    ty: TypeSchema::String,
                    comment: "RFC3339 timestamp for first thread creation.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the resulting thread summary.",
                required: true,
            }],
        },
        "create_new" => ControllerSchema {
            namespace: "threads",
            function: "create_new",
            description: "Create a new conversation thread with auto-generated ID and title.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the created thread summary.",
                required: true,
            }],
        },
        "messages_list" => ControllerSchema {
            namespace: "threads",
            function: "messages_list",
            description: "List messages for a conversation thread.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with messages and count.",
                required: true,
            }],
        },
        "message_append" => ControllerSchema {
            namespace: "threads",
            function: "message_append",
            description: "Append a message to a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "message",
                    ty: TypeSchema::Json,
                    comment: "Message payload to append.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the appended message payload.",
                required: true,
            }],
        },
        "message_update" => ControllerSchema {
            namespace: "threads",
            function: "message_update",
            description: "Patch metadata on an existing conversation message.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "message_id",
                    ty: TypeSchema::String,
                    comment: "Message identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "extra_metadata",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Replacement message metadata object.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the updated message payload.",
                required: true,
            }],
        },
        "delete" => ControllerSchema {
            namespace: "threads",
            function: "delete",
            description: "Delete a conversation thread and its message log.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "deleted_at",
                    ty: TypeSchema::String,
                    comment: "RFC3339 deletion timestamp.",
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
        "purge" => ControllerSchema {
            namespace: "threads",
            function: "purge",
            description: "Remove all conversation threads and messages.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with deleted thread/message counts.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "threads",
            function: "unknown",
            description: "Unknown threads controller function.",
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

// ── Handlers ─────────────────────────────────────────────────────────

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::threads_list(EmptyRequest {}).await?) })
}

fn handle_upsert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpsertConversationThreadRequest>(params)?;
        to_json(ops::thread_upsert(p).await?)
    })
}

fn handle_create_new(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::thread_create_new(EmptyRequest {}).await?) })
}

fn handle_messages_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ConversationMessagesRequest>(params)?;
        to_json(ops::messages_list(p).await?)
    })
}

fn handle_message_append(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<AppendConversationMessageRequest>(params)?;
        to_json(ops::message_append(p).await?)
    })
}

fn handle_message_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateConversationMessageRequest>(params)?;
        to_json(ops::message_update(p).await?)
    })
}

fn handle_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<DeleteConversationThreadRequest>(params)?;
        to_json(ops::thread_delete(p).await?)
    })
}

fn handle_purge(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::threads_purge(EmptyRequest {}).await?) })
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: crate::rpc::RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_FUNCTIONS: &[&str] = &[
        "list",
        "upsert",
        "create_new",
        "messages_list",
        "message_append",
        "message_update",
        "delete",
        "purge",
    ];

    #[test]
    fn all_controller_schemas_has_entry_per_function() {
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
    fn every_schema_uses_threads_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "threads", "schema {} wrong namespace", s.function);
        }
    }

    #[test]
    fn unknown_function_returns_fallback() {
        let s = schemas("no_such_fn");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "threads");
    }
}
