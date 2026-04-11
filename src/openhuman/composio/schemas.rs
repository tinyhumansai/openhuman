//! Controller schemas + registered handlers for the Composio domain.
//!
//! Exposes the domain over the shared registry at
//! `openhuman.composio_*`:
//!   - `composio.list_toolkits`       → `openhuman.composio_list_toolkits`
//!   - `composio.list_connections`    → `openhuman.composio_list_connections`
//!   - `composio.authorize`           → `openhuman.composio_authorize`
//!   - `composio.delete_connection`   → `openhuman.composio_delete_connection`
//!   - `composio.list_tools`          → `openhuman.composio_list_tools`
//!   - `composio.execute`             → `openhuman.composio_execute`
//!   - `composio.get_user_profile`    → `openhuman.composio_get_user_profile`
//!   - `composio.sync`                → `openhuman.composio_sync`

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_toolkits"),
        schemas("list_connections"),
        schemas("authorize"),
        schemas("delete_connection"),
        schemas("list_tools"),
        schemas("execute"),
        schemas("get_user_profile"),
        schemas("sync"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_toolkits"),
            handler: handle_list_toolkits,
        },
        RegisteredController {
            schema: schemas("list_connections"),
            handler: handle_list_connections,
        },
        RegisteredController {
            schema: schemas("authorize"),
            handler: handle_authorize,
        },
        RegisteredController {
            schema: schemas("delete_connection"),
            handler: handle_delete_connection,
        },
        RegisteredController {
            schema: schemas("list_tools"),
            handler: handle_list_tools,
        },
        RegisteredController {
            schema: schemas("execute"),
            handler: handle_execute,
        },
        RegisteredController {
            schema: schemas("get_user_profile"),
            handler: handle_get_user_profile,
        },
        RegisteredController {
            schema: schemas("sync"),
            handler: handle_sync,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_toolkits" => ControllerSchema {
            namespace: "composio",
            function: "list_toolkits",
            description: "List the Composio toolkits currently enabled on the backend allowlist.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "toolkits",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Toolkit slugs enabled by the backend (e.g. gmail, notion).",
                required: true,
            }],
        },
        "list_connections" => ControllerSchema {
            namespace: "composio",
            function: "list_connections",
            description:
                "List the caller's active Composio OAuth connections filtered to the allowlist.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "connections",
                ty: TypeSchema::Json,
                comment: "Array of {id, toolkit, status, createdAt} objects.",
                required: true,
            }],
        },
        "authorize" => ControllerSchema {
            namespace: "composio",
            function: "authorize",
            description: "Begin an OAuth handoff for a toolkit and return the hosted connect URL.",
            inputs: vec![FieldSchema {
                name: "toolkit",
                ty: TypeSchema::String,
                comment: "Toolkit slug to authorize (must be in the backend allowlist).",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "connectUrl",
                    ty: TypeSchema::String,
                    comment: "Composio-hosted OAuth URL to open in a browser.",
                    required: true,
                },
                FieldSchema {
                    name: "connectionId",
                    ty: TypeSchema::String,
                    comment: "New Composio connection id created by this authorize call.",
                    required: true,
                },
            ],
        },
        "delete_connection" => ControllerSchema {
            namespace: "composio",
            function: "delete_connection",
            description: "Delete a Composio connection owned by the caller.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::String,
                comment: "Identifier of the connection to delete.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "deleted",
                ty: TypeSchema::Bool,
                comment: "True when the backend confirmed the deletion.",
                required: true,
            }],
        },
        "list_tools" => ControllerSchema {
            namespace: "composio",
            function: "list_tools",
            description:
                "List OpenAI-function-calling tool schemas for one or more Composio toolkits.",
            inputs: vec![FieldSchema {
                name: "toolkits",
                ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                comment: "Optional list of toolkit slugs to filter by. Omit to get all.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Json,
                comment: "Array of OpenAI function-calling tool schemas.",
                required: true,
            }],
        },
        "execute" => ControllerSchema {
            namespace: "composio",
            function: "execute",
            description: "Execute a Composio action (tool slug) against a connected account.",
            inputs: vec![
                FieldSchema {
                    name: "tool",
                    ty: TypeSchema::String,
                    comment: "Composio action slug, e.g. GMAIL_SEND_EMAIL.",
                    required: true,
                },
                FieldSchema {
                    name: "arguments",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Tool-specific arguments conforming to the tool's JSON schema.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Execution envelope: { data, successful, error?, costUsd }.",
                required: true,
            }],
        },
        "get_user_profile" => ControllerSchema {
            namespace: "composio",
            function: "get_user_profile",
            description:
                "Fetch a normalized user profile for a Composio connection by dispatching to \
                 the toolkit's native provider implementation.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::String,
                comment: "Composio connection id (from list_connections / authorize).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "profile",
                ty: TypeSchema::Json,
                comment: "Normalized profile: { toolkit, connectionId, displayName?, email?, \
                          username?, avatarUrl?, extras }.",
                required: true,
            }],
        },
        "sync" => ControllerSchema {
            namespace: "composio",
            function: "sync",
            description:
                "Run a sync pass for a Composio connection by dispatching to the toolkit's \
                 native provider implementation. Persists results into the memory layer.",
            inputs: vec![
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::String,
                    comment: "Composio connection id (from list_connections / authorize).",
                    required: true,
                },
                FieldSchema {
                    name: "reason",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Optional reason: 'manual' (default), 'periodic', 'connection_created'.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "outcome",
                ty: TypeSchema::Json,
                comment: "SyncOutcome: { toolkit, connectionId, reason, itemsIngested, \
                          startedAtMs, finishedAtMs, summary, details }.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "composio",
            function: "unknown",
            description: "Unknown composio controller function.",
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

// ── Handlers ────────────────────────────────────────────────────────

fn handle_list_toolkits(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_list_toolkits(&config).await?)
    })
}

fn handle_list_connections(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_list_connections(&config).await?)
    })
}

fn handle_authorize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let toolkit = read_required_non_empty(&params, "toolkit")?;
        to_json(super::ops::composio_authorize(&config, &toolkit).await?)
    })
}

fn handle_delete_connection(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(super::ops::composio_delete_connection(&config, &connection_id).await?)
    })
}

fn handle_list_tools(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let toolkits = read_optional::<Vec<String>>(&params, "toolkits")?;
        to_json(super::ops::composio_list_tools(&config, toolkits).await?)
    })
}

fn handle_execute(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let tool = read_required_non_empty(&params, "tool")?;
        let arguments = read_optional::<Value>(&params, "arguments")?;
        to_json(super::ops::composio_execute(&config, &tool, arguments).await?)
    })
}

fn handle_get_user_profile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(super::ops::composio_get_user_profile(&config, &connection_id).await?)
    })
}

fn handle_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        let reason = read_optional::<String>(&params, "reason")?;
        to_json(super::ops::composio_sync(&config, &connection_id, reason).await?)
    })
}

// ── Param helpers ───────────────────────────────────────────────────

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

/// Read a required `String` parameter and reject blank / whitespace-only
/// input at the RPC boundary instead of letting it reach the backend.
/// Returns the trimmed value.
fn read_required_non_empty(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    let raw = read_required::<String>(params, key)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("'{key}' must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
