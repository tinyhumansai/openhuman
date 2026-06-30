//! RPC schemas and controller registration for conversation threads.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::task_board::{TaskBoard, TaskBoardCard, TaskBoardStore};
use crate::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessagesRequest, CreateConversationThreadRequest,
    DeleteConversationThreadRequest, EmptyRequest, GenerateConversationThreadTitleRequest,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest, UpsertConversationThreadRequest,
};
use crate::openhuman::threads::turn_state::{ClearTurnStateRequest, GetTurnStateRequest};

use super::ops;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("upsert"),
        schemas("create_new"),
        schemas("messages_list"),
        schemas("message_append"),
        schemas("generate_title"),
        schemas("update_labels"),
        schemas("update_title"),
        schemas("message_update"),
        schemas("delete"),
        schemas("purge"),
        schemas("turn_state_get"),
        schemas("turn_state_list"),
        schemas("turn_state_clear"),
        schemas("task_board_get"),
        schemas("task_board_put"),
        schemas("token_usage"),
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
            schema: schemas("update_labels"),
            handler: handle_update_labels,
        },
        RegisteredController {
            schema: schemas("update_title"),
            handler: handle_update_title,
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
        RegisteredController {
            schema: schemas("turn_state_get"),
            handler: handle_turn_state_get,
        },
        RegisteredController {
            schema: schemas("turn_state_list"),
            handler: handle_turn_state_list,
        },
        RegisteredController {
            schema: schemas("turn_state_clear"),
            handler: handle_turn_state_clear,
        },
        RegisteredController {
            schema: schemas("task_board_get"),
            handler: handle_task_board_get,
        },
        RegisteredController {
            schema: schemas("task_board_put"),
            handler: handle_task_board_put,
        },
        RegisteredController {
            schema: schemas("token_usage"),
            handler: handle_token_usage,
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
                FieldSchema {
                    name: "labels",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional list of labels to assign.",
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
        "create_new" => ControllerSchema {
            namespace: "threads",
            function: "create_new",
            description: "Create a new conversation thread with auto-generated ID and title.",
            inputs: vec![FieldSchema {
                name: "labels",
                ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                comment: "Optional labels to assign to the new thread.",
                required: false,
            }],
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
        "update_labels" => ControllerSchema {
            namespace: "threads",
            function: "update_labels",
            description: "Update labels for a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "labels",
                    ty: TypeSchema::Json,
                    comment: "List of labels to assign.",
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
        "update_title" => ControllerSchema {
            namespace: "threads",
            function: "update_title",
            description: "Set a user-specified title on a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "New title for the thread.",
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
        "turn_state_get" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_get",
            description: "Fetch the persisted in-flight turn snapshot for a thread, if any.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope wrapping the turn state (may be null).",
                required: true,
            }],
        },
        "turn_state_list" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_list",
            description:
                "List every persisted turn snapshot — used to surface interrupted turns on cold boot.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the list of turn snapshots and a count.",
                required: true,
            }],
        },
        "turn_state_clear" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_clear",
            description: "Delete the persisted turn snapshot for a thread.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope reporting whether a snapshot was removed.",
                required: true,
            }],
        },
        "task_board_get" => ControllerSchema {
            namespace: "threads",
            function: "task_board_get",
            description: "Fetch the persisted kanban task board for a conversation thread.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "taskBoard",
                ty: TypeSchema::Json,
                comment: "Task board payload.",
                required: true,
            }],
        },
        "task_board_put" => ControllerSchema {
            namespace: "threads",
            function: "task_board_put",
            description: "Replace the persisted kanban task board for a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "cards",
                    ty: TypeSchema::Json,
                    comment: "Array of task board cards.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "taskBoard",
                ty: TypeSchema::Json,
                comment: "Task board payload.",
                required: true,
            }],
        },
        "token_usage" => ControllerSchema {
            namespace: "threads",
            function: "token_usage",
            description: "Total a thread's persisted token/cost usage from its session transcripts.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the thread's token/cost totals (zeros when no turns yet).",
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

fn handle_create_new(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<CreateConversationThreadRequest>(params)?;
        to_json(ops::thread_create_new(p).await?)
    })
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

fn handle_update_labels(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateConversationThreadLabelsRequest>(params)?;
        to_json(ops::thread_update_labels(p).await?)
    })
}

fn handle_update_title(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateConversationThreadTitleRequest>(params)?;
        to_json(ops::thread_update_title(p).await?)
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

fn handle_turn_state_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<GetTurnStateRequest>(params)?;
        to_json(ops::turn_state_get(p).await?)
    })
}

fn handle_turn_state_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::turn_state_list(EmptyRequest {}).await?) })
}

fn handle_turn_state_clear(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ClearTurnStateRequest>(params)?;
        to_json(ops::turn_state_clear(p).await?)
    })
}

#[derive(serde::Deserialize)]
struct TaskBoardGetParams {
    thread_id: String,
}

#[derive(serde::Deserialize)]
struct TaskBoardPutParams {
    thread_id: String,
    cards: Vec<TaskBoardCard>,
}

fn handle_task_board_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<TaskBoardGetParams>(params)?;
        let thread_id = p.thread_id.trim().to_string();
        tracing::debug!(thread_id = %thread_id, "[rpc][task_board] get entry");
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| {
                tracing::debug!(
                    thread_id = %thread_id,
                    error = %e,
                    "[rpc][task_board] get load_config_error"
                );
                format!("load config: {e}")
            })?;
        tracing::trace!(thread_id = %thread_id, "[rpc][task_board] get loading_board");
        let board = crate::openhuman::agent::task_board::board_for_thread(
            &config.workspace_dir,
            &thread_id,
        )
        .map_err(|e| {
            tracing::debug!(
                thread_id = %thread_id,
                error = %e,
                "[rpc][task_board] get board_error"
            );
            e
        })?;
        tracing::debug!(
            thread_id = %thread_id,
            card_count = board.cards.len(),
            "[rpc][task_board] get exit"
        );
        Ok(serde_json::json!({ "taskBoard": board }))
    })
}

fn handle_task_board_put(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<TaskBoardPutParams>(params)?;
        let thread_id = p.thread_id.trim().to_string();
        tracing::debug!(
            thread_id = %thread_id,
            card_count = p.cards.len(),
            "[rpc][task_board] put entry"
        );
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| {
                tracing::debug!(
                    thread_id = %thread_id,
                    error = %e,
                    "[rpc][task_board] put load_config_error"
                );
                format!("load config: {e}")
            })?;
        let board = TaskBoard {
            thread_id: thread_id.clone(),
            cards: p.cards,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let saved = TaskBoardStore::new(config.workspace_dir)
            .put(board)
            .map_err(|e| {
                tracing::debug!(
                    thread_id = %thread_id,
                    error = %e,
                    "[rpc][task_board] put store_error"
                );
                e
            })?;
        tracing::debug!(
            thread_id = %thread_id,
            card_count = saved.cards.len(),
            "[rpc][task_board] put exit"
        );
        Ok(serde_json::json!({ "taskBoard": saved }))
    })
}

fn handle_token_usage(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::ThreadTokenUsageRequest>(params)?;
        to_json(ops::token_usage(p).await?)
    })
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: crate::rpc::RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
