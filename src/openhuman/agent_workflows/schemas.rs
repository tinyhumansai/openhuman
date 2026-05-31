//! Controller schemas for the `workflows` RPC namespace.
//!
//! Methods: `openhuman.workflows_list`, `workflows_read`, `workflows_create`,
//! `workflows_uninstall`, `workflows_phase`. Mirrors the cron/skills controller
//! shape (`schemas()` / `all_controller_schemas()` / `all_registered_controllers()`
//! / `handle_*`) using `RpcOutcome` + `into_cli_compatible_json()`.

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use super::WorkflowSummary;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("read"),
        schemas("create"),
        schemas("uninstall"),
        schemas("phase"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("read"),
            handler: handle_read,
        },
        RegisteredController {
            schema: schemas("create"),
            handler: handle_create,
        },
        RegisteredController {
            schema: schemas("uninstall"),
            handler: handle_uninstall,
        },
        RegisteredController {
            schema: schemas("phase"),
            handler: handle_phase,
        },
    ]
}

fn id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "workflows",
            function: "list",
            description: "List all discovered workflows (user scope).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "workflows",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("WorkflowSummary"))),
                comment: "Discovered workflows.",
                required: true,
            }],
        },
        "read" => ControllerSchema {
            namespace: "workflows",
            function: "read",
            description: "Read a single workflow (frontmatter + body) by id.",
            inputs: vec![id_input("Workflow id (directory name).")],
            outputs: vec![FieldSchema {
                name: "workflow",
                ty: TypeSchema::Ref("Workflow"),
                comment: "The resolved workflow.",
                required: true,
            }],
        },
        "create" => ControllerSchema {
            namespace: "workflows",
            function: "create",
            description: "Scaffold a new user-scope workflow (WORKFLOW.md).",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Human-readable workflow name.",
                    required: true,
                },
                FieldSchema {
                    name: "description",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Short description.",
                    required: false,
                },
                FieldSchema {
                    name: "when_to_use",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Free-text trigger used for auto-match.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "workflow",
                ty: TypeSchema::Ref("Workflow"),
                comment: "Newly created workflow.",
                required: true,
            }],
        },
        "uninstall" => ControllerSchema {
            namespace: "workflows",
            function: "uninstall",
            description: "Remove a user-scope workflow by id.",
            inputs: vec![id_input("Workflow id (directory name) to remove.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "id",
                            ty: TypeSchema::String,
                            comment: "Requested id.",
                            required: true,
                        },
                        FieldSchema {
                            name: "removed",
                            ty: TypeSchema::Bool,
                            comment: "True when removed.",
                            required: true,
                        },
                    ],
                },
                comment: "Removal result.",
                required: true,
            }],
        },
        "phase" => ControllerSchema {
            namespace: "workflows",
            function: "phase",
            description: "Resolve a phase's guidance + effective tool scope for a workflow.",
            inputs: vec![
                id_input("Workflow id (directory name)."),
                FieldSchema {
                    name: "phase",
                    ty: TypeSchema::String,
                    comment: "Phase name (e.g. on_pick_up_task).",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "guidance",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "Rendered rules block, if any.",
                            required: false,
                        },
                        FieldSchema {
                            name: "tool_scope",
                            ty: TypeSchema::Option(Box::new(TypeSchema::Ref("ToolScope"))),
                            comment: "Effective tool scope, if any.",
                            required: false,
                        },
                    ],
                },
                comment: "Resolved phase payload.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "workflows",
            function: "unknown",
            description: "Unknown workflows controller function.",
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

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[workflows][rpc] list");
        let workflows = super::discover_workflows(dirs::home_dir().as_deref(), None, false);
        let summaries: Vec<WorkflowSummary> = workflows.iter().map(WorkflowSummary::from).collect();
        log::debug!("[workflows][rpc] list -> {} workflow(s)", summaries.len());
        to_json(RpcOutcome::new(
            json!({ "workflows": summaries }),
            Vec::new(),
        ))
    })
}

fn handle_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = read_required::<String>(&params, "id")?;
        log::debug!("[workflows][rpc] read id={id}");
        let workflow = super::read_workflow(id.trim())?;
        to_json(RpcOutcome::new(json!({ "workflow": workflow }), Vec::new()))
    })
}

fn handle_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let name = read_required::<String>(&params, "name")?;
        let description = read_optional_str(&params, "description").unwrap_or_default();
        let when_to_use = read_optional_str(&params, "when_to_use").unwrap_or_default();
        log::info!("[workflows][rpc] create name={name}");
        let workflow = super::create_workflow(name.trim(), description.trim(), when_to_use.trim())?;
        to_json(RpcOutcome::new(json!({ "workflow": workflow }), Vec::new()))
    })
}

fn handle_uninstall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = read_required::<String>(&params, "id")?;
        log::info!("[workflows][rpc] uninstall id={id}");
        let removed = super::uninstall_workflow(id.trim())?;
        to_json(RpcOutcome::new(
            json!({ "id": id.trim(), "removed": removed }),
            Vec::new(),
        ))
    })
}

fn handle_phase(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = read_required::<String>(&params, "id")?;
        let phase = read_required::<String>(&params, "phase")?;
        log::debug!("[workflows][rpc] phase id={id} phase={phase}");
        let workflow = super::read_workflow(id.trim())?;
        let guidance = super::phase_guidance(&workflow, phase.trim());
        let tool_scope = super::effective_tool_scope(&workflow, phase.trim());
        to_json(RpcOutcome::new(
            json!({ "guidance": guidance, "tool_scope": tool_scope }),
            Vec::new(),
        ))
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional_str(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
