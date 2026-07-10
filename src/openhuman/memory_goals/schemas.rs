//! Controller schemas + JSON-RPC handlers for the `memory_goals` namespace.
//!
//! Methods are exposed as `openhuman.memory_goals_<function>`:
//! `list`, `add`, `edit`, `delete`, `reflect`. Handlers load the active
//! config (for `workspace_dir`), delegate to [`super::ops`], and serialise
//! the [`RpcOutcome`] into the CLI-compatible JSON shape.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::ops;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

/// All `memory_goals` controller schemas (advertised to CLI + RPC consumers).
pub fn all_memory_goals_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("add"),
        schemas("edit"),
        schemas("delete"),
        schemas("reflect"),
    ]
}

/// Registered `memory_goals` controllers (schema + handler pairs).
pub fn all_memory_goals_registered_controllers() -> Vec<RegisteredController> {
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
            schema: schemas("delete"),
            handler: handle_delete,
        },
        RegisteredController {
            schema: schemas("reflect"),
            handler: handle_reflect,
        },
    ]
}

/// Schema definitions for every `memory_goals` function.
fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "memory_goals",
            function: "list",
            description: "List the agent's long-term goals for working with the user.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "items",
                ty: TypeSchema::Json,
                comment: "The current goals as a bare document: { items: [{ id, text }] }.",
                required: true,
            }],
        },
        "add" => ControllerSchema {
            namespace: "memory_goals",
            function: "add",
            description: "Add a new long-term goal item.",
            inputs: vec![FieldSchema {
                name: "text",
                ty: TypeSchema::String,
                comment: "The goal text — one concise sentence.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{ id, goals } — assigned id plus the updated list.",
                required: true,
            }],
        },
        "edit" => ControllerSchema {
            namespace: "memory_goals",
            function: "edit",
            description: "Edit an existing long-term goal by id.",
            inputs: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "The goal id to edit (e.g. 'g1').",
                    required: true,
                },
                FieldSchema {
                    name: "text",
                    ty: TypeSchema::String,
                    comment: "The new goal text.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "CLI-envelope { result: { items }, logs } — the updated list.",
                required: true,
            }],
        },
        "delete" => ControllerSchema {
            namespace: "memory_goals",
            function: "delete",
            description: "Delete a long-term goal by id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "The goal id to delete (e.g. 'g1').",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "CLI-envelope { result: { items }, logs } — the updated list.",
                required: true,
            }],
        },
        "reflect" => ControllerSchema {
            namespace: "memory_goals",
            function: "reflect",
            description: "Run the goals enrichment agent now and return the updated list.",
            inputs: vec![FieldSchema {
                name: "context",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment:
                    "Optional context/prompt to enrich from (defaults to a generic review nudge).",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{ ran, summary, goals } — outcome of the enrichment pass.",
                required: true,
            }],
        },
        other => panic!("unknown memory_goals function: {other}"),
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::list(&config.workspace_dir).await?)
    })
}

fn handle_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<AddParams>(Value::Object(params))?;
        to_json(ops::add(&config.workspace_dir, &req.text).await?)
    })
}

fn handle_edit(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<EditParams>(Value::Object(params))?;
        to_json(ops::edit(&config.workspace_dir, &req.id, &req.text).await?)
    })
}

fn handle_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<DeleteParams>(Value::Object(params))?;
        to_json(ops::delete(&config.workspace_dir, &req.id).await?)
    })
}

fn handle_reflect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<ReflectParams>(Value::Object(params))?;
        to_json(ops::reflect_now(&config, req.context).await?)
    })
}

// ── Param structs + helpers ──────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct AddParams {
    text: String,
}

#[derive(serde::Deserialize)]
struct EditParams {
    id: String,
    text: String,
}

#[derive(serde::Deserialize)]
struct DeleteParams {
    id: String,
}

#[derive(serde::Deserialize)]
struct ReflectParams {
    #[serde(default)]
    context: Option<String>,
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_all_five_controllers() {
        let controllers = all_memory_goals_registered_controllers();
        assert_eq!(controllers.len(), 5);
        let methods: Vec<String> = controllers
            .iter()
            .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
            .collect();
        for expected in [
            "memory_goals.list",
            "memory_goals.add",
            "memory_goals.edit",
            "memory_goals.delete",
            "memory_goals.reflect",
        ] {
            assert!(
                methods.contains(&expected.to_string()),
                "missing {expected}"
            );
        }
    }

    #[test]
    fn schemas_and_controllers_stay_in_sync() {
        assert_eq!(
            all_memory_goals_controller_schemas().len(),
            all_memory_goals_registered_controllers().len()
        );
    }
}
