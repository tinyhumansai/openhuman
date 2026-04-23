//! RPC schemas and controller registration for conversation threads.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessagesRequest, DeleteConversationThreadRequest,
    EmptyRequest, GenerateConversationThreadTitleRequest, UpdateConversationMessageRequest,
    UpsertConversationThreadRequest,
};

use super::ops;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("upsert"),
        schemas("create_new"),
        schemas("messages_list"),
        schemas("message_append"),
        schemas("generate_title"),
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
            schema: schemas("generate_title"),
            handler: handle_generate_title,
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
        "generate_title" => ControllerSchema {
            namespace: "threads",
            function: "generate_title",
            description:
                "Generate a short thread title from the first user message and assistant reply.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "assistant_message",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Optional completed assistant reply to use instead of the stored first agent message.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the resulting thread summary.",
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

fn handle_generate_title(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<GenerateConversationThreadTitleRequest>(params)?;
        to_json(ops::thread_generate_title(p).await?)
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
    use serde_json::json;

    const ALL_FUNCTIONS: &[&str] = &[
        "list",
        "upsert",
        "create_new",
        "messages_list",
        "message_append",
        "generate_title",
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
            assert_eq!(
                s.namespace, "threads",
                "schema {} wrong namespace",
                s.function
            );
        }
    }

    #[test]
    fn unknown_function_returns_fallback() {
        let s = schemas("no_such_fn");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "threads");
    }

    // ── parse::<T>(params) contract ─────────────────────────────────────

    fn obj(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(m) => m,
            _ => panic!("expected JSON object"),
        }
    }

    #[test]
    fn parse_upsert_accepts_snake_case_contract() {
        let p: UpsertConversationThreadRequest = parse(obj(json!({
            "id": "t1",
            "title": "Hello",
            "created_at": "2026-04-22T00:00:00Z",
        })))
        .expect("valid snake_case params parse");
        assert_eq!(p.id, "t1");
        assert_eq!(p.title, "Hello");
        assert_eq!(p.created_at, "2026-04-22T00:00:00Z");
    }

    #[test]
    fn parse_upsert_rejects_camel_case_created_at() {
        // Request params contract is snake_case; camelCase must not silently
        // succeed because `createdAt` leaves `created_at` missing and also
        // trips deny_unknown_fields.
        let err = parse::<UpsertConversationThreadRequest>(obj(json!({
            "id": "t1",
            "title": "Hello",
            "createdAt": "2026-04-22T00:00:00Z",
        })))
        .unwrap_err();
        assert!(err.starts_with("invalid params:"), "prefix: {err}");
    }

    #[test]
    fn parse_upsert_rejects_unknown_fields() {
        let err = parse::<UpsertConversationThreadRequest>(obj(json!({
            "id": "t1",
            "title": "Hello",
            "created_at": "2026-04-22T00:00:00Z",
            "extra": "nope",
        })))
        .unwrap_err();
        assert!(err.starts_with("invalid params:"), "prefix: {err}");
        assert!(err.contains("extra"), "field name in error: {err}");
    }

    #[test]
    fn parse_upsert_missing_required_field_errors() {
        let err = parse::<UpsertConversationThreadRequest>(obj(json!({
            "id": "t1",
            "title": "Hello",
        })))
        .unwrap_err();
        assert!(err.starts_with("invalid params:"), "prefix: {err}");
        assert!(err.contains("created_at"), "field name in error: {err}");
    }

    #[test]
    fn parse_messages_list_requires_thread_id() {
        let ok: ConversationMessagesRequest = parse(obj(json!({"thread_id": "t1"}))).unwrap();
        assert_eq!(ok.thread_id, "t1");

        let err = parse::<ConversationMessagesRequest>(obj(json!({}))).unwrap_err();
        assert!(err.contains("thread_id"), "err: {err}");

        // camelCase alias is not accepted under deny_unknown_fields.
        let err = parse::<ConversationMessagesRequest>(obj(json!({"threadId": "t1"}))).unwrap_err();
        assert!(err.starts_with("invalid params:"), "prefix: {err}");
    }

    #[test]
    fn parse_message_append_nested_message_requires_camel_case() {
        // Outer request is snake_case; nested ConversationMessageRecord is
        // camelCase by contract (messageType / createdAt). Assert both paths.
        let ok: AppendConversationMessageRequest = parse(obj(json!({
            "thread_id": "t1",
            "message": {
                "id": "m1",
                "content": "hi",
                "type": "text",
                "sender": "user",
                "createdAt": "2026-04-22T00:00:00Z",
            }
        })))
        .expect("valid nested camelCase message");
        assert_eq!(ok.thread_id, "t1");
        assert_eq!(ok.message.id, "m1");
        assert_eq!(ok.message.created_at, "2026-04-22T00:00:00Z");

        let err = parse::<AppendConversationMessageRequest>(obj(json!({
            "thread_id": "t1",
            "message": {
                "id": "m1",
                "content": "hi",
                "type": "text",
                "sender": "user",
                "created_at": "2026-04-22T00:00:00Z",
            }
        })))
        .unwrap_err();
        assert!(
            err.contains("createdAt"),
            "err surfaces expected key: {err}"
        );
    }

    #[test]
    fn parse_generate_title_assistant_message_is_optional() {
        let without: GenerateConversationThreadTitleRequest =
            parse(obj(json!({"thread_id": "t1"}))).unwrap();
        assert_eq!(without.thread_id, "t1");
        assert_eq!(without.assistant_message, None);

        let with: GenerateConversationThreadTitleRequest = parse(obj(json!({
            "thread_id": "t1",
            "assistant_message": "reply",
        })))
        .unwrap();
        assert_eq!(with.assistant_message.as_deref(), Some("reply"));
    }

    #[test]
    fn parse_message_update_extra_metadata_optional_and_unknown_rejected() {
        let without: UpdateConversationMessageRequest = parse(obj(json!({
            "thread_id": "t1",
            "message_id": "m1",
        })))
        .unwrap();
        assert!(without.extra_metadata.is_none());

        let with: UpdateConversationMessageRequest = parse(obj(json!({
            "thread_id": "t1",
            "message_id": "m1",
            "extra_metadata": {"k": "v"},
        })))
        .unwrap();
        assert_eq!(with.extra_metadata, Some(json!({"k": "v"})));

        let err = parse::<UpdateConversationMessageRequest>(obj(json!({
            "thread_id": "t1",
            "message_id": "m1",
            "bogus": true,
        })))
        .unwrap_err();
        assert!(err.contains("bogus"), "err: {err}");
    }

    #[test]
    fn parse_delete_requires_thread_id_and_deleted_at() {
        let ok: DeleteConversationThreadRequest = parse(obj(json!({
            "thread_id": "t1",
            "deleted_at": "2026-04-22T00:00:00Z",
        })))
        .unwrap();
        assert_eq!(ok.thread_id, "t1");

        let err =
            parse::<DeleteConversationThreadRequest>(obj(json!({"thread_id": "t1"}))).unwrap_err();
        assert!(err.contains("deleted_at"), "err: {err}");
    }

    #[test]
    fn parse_empty_request_rejects_any_field() {
        let _: EmptyRequest = parse(obj(json!({}))).unwrap();
        let err = parse::<EmptyRequest>(obj(json!({"x": 1}))).unwrap_err();
        assert!(err.starts_with("invalid params:"), "prefix: {err}");
    }
}
