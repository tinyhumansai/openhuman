//! Controller schemas and RPC handlers for the `socket` namespace.

use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::manager::global_socket_manager;
use crate::api::models::socket::{ConnectionStatus, SocketState};

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

/// Serialise a [`ConnectionStatus`] the way every other surface publishes it.
///
/// `socket_state` (`serde_json::to_value(state)`) and `connectivity_diag` both go through
/// `ConnectionStatus`' `#[serde(rename_all = "lowercase")]`, so the lifecycle handlers below use
/// the same encoding instead of `format!("{:?}", …)` (#6111). `Debug` and serde disagree —
/// `"Disconnected"` vs `"disconnected"` — and one namespace publishing a field two ways is a
/// contract a caller cannot rely on.
fn status_payload(state: &SocketState) -> Value {
    json!({ "status": Value::String(status_slug(state.status)) })
}

/// The serde spelling of `status`, as a plain string.
///
/// Goes through `serde_json::to_value` rather than a hand-written match so the mapping cannot
/// drift from the `rename_all` attribute that `socket_state` relies on. `ConnectionStatus` is a
/// unit-variant enum, so this cannot fail; the fallback keeps the function total.
fn status_slug(status: ConnectionStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}").to_lowercase())
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

        let state = super::ops::connect_static(mgr, url, token).await?;
        Ok(status_payload(&state))
    })
}

fn handle_disconnect(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let state = super::ops::disconnect(mgr).await?;
        Ok(status_payload(&state))
    })
}

fn handle_state(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let state = mgr.get_state();
        log::debug!("[socket:rpc] state → {:?}", state.status);
        serde_json::to_value(state).map_err(|e| format!("serialize: {e}"))
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
        let state = super::ops::connect_with_session(mgr).await?;
        Ok(status_payload(&state))
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
