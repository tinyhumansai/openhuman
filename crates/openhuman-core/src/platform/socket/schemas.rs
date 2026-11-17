//! Controller schemas and RPC handlers for the `socket` namespace.

use crate::core::runtime::DispatchCtx;
use serde_json::{json, Map, Value};

use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, ControllerTask,
    RegisteredController,
};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::global_socket_manager;
use crate::platform::socket::models::{ConnectionStatus, SocketState};

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

/// Runtime registrations share each decoder with the compatible legacy handlers below.
pub(crate) fn all_controller_entries() -> Vec<ControllerEntry> {
    vec![
        connect_entry(),
        disconnect_entry(),
        state_entry(),
        emit_entry(),
        connect_with_session_entry(),
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

fn require_manager() -> Result<std::sync::Arc<dyn super::SocketPort>, String> {
    global_socket_manager()
        .ok_or_else(|| "SocketManager not initialized — runtime not bootstrapped".to_string())
}

type SocketPortRef = std::sync::Arc<dyn super::SocketPort>;

struct ConnectParams {
    url: String,
    token: String,
}

struct EmitParams {
    event: String,
    data: Value,
}

fn gate_manager(_ctx: DispatchCtx) -> ControllerTask<SocketPortRef> {
    Box::pin(async { require_manager() })
}

fn parse_connect(params: Map<String, Value>) -> Result<ConnectParams, String> {
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .ok_or("missing required param 'url'")?;
    let token = params
        .get("token")
        .and_then(Value::as_str)
        .ok_or("missing required param 'token'")?;
    Ok(ConnectParams {
        url: url.to_owned(),
        token: token.to_owned(),
    })
}

fn parse_emit(params: Map<String, Value>) -> Result<EmitParams, String> {
    let event = params
        .get("event")
        .and_then(Value::as_str)
        .ok_or("missing required param 'event'")?;
    let data = params.get("data").cloned().unwrap_or(Value::Null);
    Ok(EmitParams {
        event: event.to_owned(),
        data,
    })
}

fn parse_ignored_params(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn connect_entry() -> ControllerEntry {
    ControllerEntry::bound(BoundRegisteredController::after_gate(
        schemas("connect"),
        gate_manager,
        parse_connect,
        run_connect,
    ))
}

fn disconnect_entry() -> ControllerEntry {
    ControllerEntry::bound(BoundRegisteredController::after_gate(
        schemas("disconnect"),
        gate_manager,
        parse_ignored_params,
        run_disconnect,
    ))
}

fn state_entry() -> ControllerEntry {
    ControllerEntry::bound(BoundRegisteredController::after_gate(
        schemas("state"),
        gate_manager,
        parse_ignored_params,
        run_state,
    ))
}

fn emit_entry() -> ControllerEntry {
    ControllerEntry::bound(BoundRegisteredController::after_gate(
        schemas("emit"),
        gate_manager,
        parse_emit,
        run_emit,
    ))
}

fn connect_with_session_entry() -> ControllerEntry {
    ControllerEntry::bound(BoundRegisteredController::after_gate(
        schemas("connect_with_session"),
        gate_manager,
        parse_ignored_params,
        run_connect_with_session,
    ))
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

fn run_connect(
    _ctx: DispatchCtx,
    manager: SocketPortRef,
    params: ConnectParams,
) -> ControllerFuture {
    Box::pin(async move {
        let state =
            super::ops::connect_static(manager.as_ref(), &params.url, &params.token).await?;
        Ok(status_payload(&state))
    })
}

fn run_disconnect(_ctx: DispatchCtx, manager: SocketPortRef, _params: ()) -> ControllerFuture {
    Box::pin(async move {
        let state = super::ops::disconnect(manager.as_ref()).await?;
        Ok(status_payload(&state))
    })
}

fn run_state(_ctx: DispatchCtx, manager: SocketPortRef, _params: ()) -> ControllerFuture {
    Box::pin(async move {
        let state = manager.get_state();
        log::debug!("[socket:rpc] state → {:?}", state.status);
        serde_json::to_value(state).map_err(|e| format!("serialize: {e}"))
    })
}

fn run_emit(_ctx: DispatchCtx, manager: SocketPortRef, params: EmitParams) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[socket:rpc] emit event={}", params.event);
        manager.emit(&params.event, params.data).await?;
        Ok(json!({ "ok": true }))
    })
}

fn run_connect_with_session(
    ctx: DispatchCtx,
    manager: SocketPortRef,
    _params: (),
) -> ControllerFuture {
    Box::pin(async move {
        let state = super::ops::connect_with_session(&ctx, manager.as_ref()).await?;
        Ok(status_payload(&state))
    })
}

fn handle_connect(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    connect_entry().invoke(ctx, params)
}

fn handle_disconnect(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    disconnect_entry().invoke(ctx, params)
}

fn handle_state(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    state_entry().invoke(ctx, params)
}

fn handle_emit(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    emit_entry().invoke(ctx, params)
}

fn handle_connect_with_session(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    connect_with_session_entry().invoke(ctx, params)
}

#[cfg(test)]
pub(crate) fn socket_parameter_contracts() -> Vec<crate::core::all::ControllerParamContract> {
    tests::socket_parameter_contracts()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
