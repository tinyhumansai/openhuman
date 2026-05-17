//! JSON-RPC surface for the per-thread todo list. Pairs with the agent
//! `todo` tool — both call into [`super::ops`] so user-driven and
//! agent-driven edits share the exact same persistence and rendering
//! logic.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::task_board::TaskBoardCard;

use super::ops::{self, BoardLocation, CardPatch, TodosSnapshot};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("add"),
        schemas("edit"),
        schemas("update_status"),
        schemas("remove"),
        schemas("replace"),
        schemas("clear"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("add"),
            handler: handle_add,
        },
        RegisteredController {
            schema: schemas("edit"),
            handler: handle_edit,
        },
        RegisteredController {
            schema: schemas("update_status"),
            handler: handle_update_status,
        },
        RegisteredController {
            schema: schemas("remove"),
            handler: handle_remove,
        },
        RegisteredController {
            schema: schemas("replace"),
            handler: handle_replace,
        },
        RegisteredController {
            schema: schemas("clear"),
            handler: handle_clear,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "todos",
            function: "list",
            description:
                "Return the current todo list for a conversation thread as cards + markdown.",
            inputs: vec![thread_id_input()],
            outputs: vec![snapshot_output()],
        },
        "add" => ControllerSchema {
            namespace: "todos",
            function: "add",
            description: "Append a new todo card to a conversation thread's list.",
            inputs: vec![
                thread_id_input(),
                required_string("content", "Card title / description."),
                optional_string("status", "Initial status (todo|in_progress|blocked|done)."),
                optional_string("notes", "Free-text notes."),
                optional_string("blocker", "Reason the card is blocked, if any."),
            ],
            outputs: vec![snapshot_output()],
        },
        "edit" => ControllerSchema {
            namespace: "todos",
            function: "edit",
            description: "Edit an existing todo card by id. Any omitted field is left unchanged.",
            inputs: vec![
                thread_id_input(),
                required_string("id", "Card identifier returned by `add` / `list`."),
                optional_string("content", "New title / description."),
                optional_string("status", "New status."),
                optional_string("notes", "New notes (pass empty string to clear)."),
                optional_string(
                    "blocker",
                    "New blocker reason (pass empty string to clear).",
                ),
            ],
            outputs: vec![snapshot_output()],
        },
        "update_status" => ControllerSchema {
            namespace: "todos",
            function: "update_status",
            description: "Update only the status of a todo card.",
            inputs: vec![
                thread_id_input(),
                required_string("id", "Card identifier."),
                required_string("status", "New status (todo|in_progress|blocked|done)."),
            ],
            outputs: vec![snapshot_output()],
        },
        "remove" => ControllerSchema {
            namespace: "todos",
            function: "remove",
            description: "Remove a todo card from a thread's list.",
            inputs: vec![thread_id_input(), required_string("id", "Card identifier.")],
            outputs: vec![snapshot_output()],
        },
        "replace" => ControllerSchema {
            namespace: "todos",
            function: "replace",
            description: "Wholesale-replace the todo list for a thread.",
            inputs: vec![
                thread_id_input(),
                FieldSchema {
                    name: "cards",
                    ty: TypeSchema::Json,
                    comment: "Array of card objects (id may be empty — server generates).",
                    required: true,
                },
            ],
            outputs: vec![snapshot_output()],
        },
        "clear" => ControllerSchema {
            namespace: "todos",
            function: "clear",
            description: "Empty the todo list for a thread.",
            inputs: vec![thread_id_input()],
            outputs: vec![snapshot_output()],
        },
        _ => ControllerSchema {
            namespace: "todos",
            function: "unknown",
            description: "Unknown todos controller function.",
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

#[derive(Debug, Deserialize)]
struct ThreadIdParams {
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct AddParams {
    thread_id: String,
    content: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    blocker: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EditParams {
    thread_id: String,
    id: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    blocker: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateStatusParams {
    thread_id: String,
    id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct RemoveParams {
    thread_id: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct ReplaceParams {
    thread_id: String,
    cards: Vec<TaskBoardCard>,
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ThreadIdParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(thread_id = %p.thread_id, "[rpc][todos] list entry");
        snapshot_to_json(ops::list(&loc)?)
    })
}

fn handle_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<AddParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let patch = CardPatch {
            content: None,
            status: p.status.as_deref().map(ops::parse_status).transpose()?,
            notes: p.notes,
            blocker: p.blocker,
        };
        tracing::debug!(thread_id = %p.thread_id, "[rpc][todos] add entry");
        snapshot_to_json(ops::add(&loc, &p.content, patch)?)
    })
}

fn handle_edit(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<EditParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let patch = CardPatch {
            content: p.content,
            status: p.status.as_deref().map(ops::parse_status).transpose()?,
            notes: p.notes,
            blocker: p.blocker,
        };
        tracing::debug!(thread_id = %p.thread_id, id = %p.id, "[rpc][todos] edit entry");
        snapshot_to_json(ops::edit(&loc, &p.id, patch)?)
    })
}

fn handle_update_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateStatusParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let status = ops::parse_status(&p.status)?;
        tracing::debug!(
            thread_id = %p.thread_id,
            id = %p.id,
            status = %p.status,
            "[rpc][todos] update_status entry"
        );
        snapshot_to_json(ops::update_status(&loc, &p.id, status)?)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<RemoveParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(thread_id = %p.thread_id, id = %p.id, "[rpc][todos] remove entry");
        snapshot_to_json(ops::remove(&loc, &p.id)?)
    })
}

fn handle_replace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ReplaceParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(
            thread_id = %p.thread_id,
            card_count = p.cards.len(),
            "[rpc][todos] replace entry"
        );
        snapshot_to_json(ops::replace(&loc, p.cards)?)
    })
}

fn handle_clear(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ThreadIdParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(thread_id = %p.thread_id, "[rpc][todos] clear entry");
        snapshot_to_json(ops::clear(&loc)?)
    })
}

// ── helpers ──────────────────────────────────────────────────────────

async fn thread_location(thread_id: &str) -> Result<BoardLocation, String> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err("thread_id must not be empty".to_string());
    }
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    Ok(BoardLocation::Thread {
        workspace_dir: config.workspace_dir,
        thread_id: trimmed.to_string(),
    })
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn snapshot_to_json(snap: TodosSnapshot) -> Result<Value, String> {
    serde_json::to_value(&snap).map_err(|e| format!("serialize snapshot: {e}"))
}

fn thread_id_input() -> FieldSchema {
    FieldSchema {
        name: "thread_id",
        ty: TypeSchema::String,
        comment: "Conversation thread identifier (same id used by `threads.task_board_*`).",
        required: true,
    }
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

fn snapshot_output() -> FieldSchema {
    FieldSchema {
        name: "snapshot",
        ty: TypeSchema::Json,
        comment: "Object with `threadId`, `cards`, and a `markdown` rendering of the list.",
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }

    #[test]
    fn schemas_have_todos_namespace() {
        for schema in all_controller_schemas() {
            assert_eq!(schema.namespace, "todos", "function={}", schema.function);
        }
    }
}
