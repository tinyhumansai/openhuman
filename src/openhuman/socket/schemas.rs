//! Controller schemas and RPC handlers for the `socket` namespace.

use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::manager::global_socket_manager;

// ---------------------------------------------------------------------------
// Schema catalog
// ---------------------------------------------------------------------------

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("connect"),
        schemas("disconnect"),
        schemas("state"),
        schemas("emit"),
        schemas("connect_with_session"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("connect"),
            handler: handle_connect,
        },
        RegisteredController {
            schema: schemas("disconnect"),
            handler: handle_disconnect,
        },
        RegisteredController {
            schema: schemas("state"),
            handler: handle_state,
        },
        RegisteredController {
            schema: schemas("emit"),
            handler: handle_emit,
        },
        RegisteredController {
            schema: schemas("connect_with_session"),
            handler: handle_connect_with_session,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "connect" => ControllerSchema {
            namespace: "socket",
            function: "connect",
            description: "Connect to the backend Socket.IO server.",
            inputs: vec![
                FieldSchema {
                    name: "url",
                    ty: TypeSchema::String,
                    comment: "Backend WebSocket base URL.",
                    required: true,
                },
                FieldSchema {
                    name: "token",
                    ty: TypeSchema::String,
                    comment: "Authentication JWT token.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Connection status after initiating.",
                required: true,
            }],
        },
        "disconnect" => ControllerSchema {
            namespace: "socket",
            function: "disconnect",
            description: "Disconnect from the backend Socket.IO server.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Connection status after disconnect.",
                required: true,
            }],
        },
        "state" => ControllerSchema {
            namespace: "socket",
            function: "state",
            description: "Get the current socket connection state.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Json,
                comment: "Current socket state (status, socket_id, error).",
                required: true,
            }],
        },
        "emit" => ControllerSchema {
            namespace: "socket",
            function: "emit",
            description: "Emit a Socket.IO event to the backend server.",
            inputs: vec![
                FieldSchema {
                    name: "event",
                    ty: TypeSchema::String,
                    comment: "Event name to emit.",
                    required: true,
                },
                FieldSchema {
                    name: "data",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Event payload data.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Whether the emit succeeded.",
                required: true,
            }],
        },
        "connect_with_session" => ControllerSchema {
            namespace: "socket",
            function: "connect_with_session",
            description:
                "Connect to the backend using the stored session token and configured API URL.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Connection status after initiating.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "socket",
            function: "unknown",
            description: "Unknown socket controller function.",
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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn require_manager() -> Result<&'static std::sync::Arc<super::SocketManager>, String> {
    global_socket_manager()
        .ok_or_else(|| "SocketManager not initialized — runtime not bootstrapped".to_string())
}

fn handle_connect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or("missing required param 'url'")?;
        let token = params
            .get("token")
            .and_then(|v| v.as_str())
            .ok_or("missing required param 'token'")?;

        log::info!("[socket:rpc] connect url={}", url);
        mgr.connect(url, token).await?;

        let state = mgr.get_state();
        Ok(json!({ "status": format!("{:?}", state.status) }))
    })
}

fn handle_disconnect(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        log::info!("[socket:rpc] disconnect");
        mgr.disconnect().await?;

        let state = mgr.get_state();
        Ok(json!({ "status": format!("{:?}", state.status) }))
    })
}

fn handle_state(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let state = mgr.get_state();
        log::debug!("[socket:rpc] state → {:?}", state.status);
        Ok(serde_json::to_value(state).map_err(|e| format!("serialize: {e}"))?)
    })
}

fn handle_emit(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let event = params
            .get("event")
            .and_then(|v| v.as_str())
            .ok_or("missing required param 'event'")?;
        let data = params.get("data").cloned().unwrap_or(Value::Null);

        log::debug!("[socket:rpc] emit event={}", event);
        mgr.emit(event, data).await?;
        Ok(json!({ "ok": true }))
    })
}

fn handle_connect_with_session(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;

        log::info!("[socket:rpc] connect_with_session — resolving credentials");

        // Load config for API URL and session token.
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        let api_url = crate::api::config::effective_api_url(&config.api_url);
        let token = crate::api::jwt::get_session_token(&config)
            .map_err(|e| format!("failed to read session token: {e}"))?
            .ok_or("no session token stored — user must log in first")?;

        log::info!(
            "[socket:rpc] connect_with_session url={} token_len={}",
            api_url,
            token.len()
        );

        mgr.connect(&api_url, &token).await?;

        let state = mgr.get_state();
        Ok(json!({ "status": format!("{:?}", state.status) }))
    })
}
