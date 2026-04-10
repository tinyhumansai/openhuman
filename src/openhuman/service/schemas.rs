//! Controller schemas and registration for the service domain.
//!
//! This module defines the transport-agnostic interface for service lifecycle
//! management (install, start, stop, etc.) and provides the mapping between
//! RPC methods and their underlying implementation handlers.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

/// Returns a collection of all available controller schemas for the service domain.
///
/// These schemas describe the input parameters, output fields, and metadata for
/// every service-related RPC method.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("install"),
        schemas("restart"),
        schemas("start"),
        schemas("stop"),
        schemas("status"),
        schemas("uninstall"),
        schemas("daemon_host_get"),
        schemas("daemon_host_set"),
    ]
}

/// Returns a collection of all registered controllers for the service domain.
///
/// Each `RegisteredController` pairs a `ControllerSchema` with its corresponding
/// async handler function.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("install"),
            handler: handle_install,
        },
        RegisteredController {
            schema: schemas("restart"),
            handler: handle_restart,
        },
        RegisteredController {
            schema: schemas("start"),
            handler: handle_start,
        },
        RegisteredController {
            schema: schemas("stop"),
            handler: handle_stop,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("uninstall"),
            handler: handle_uninstall,
        },
        RegisteredController {
            schema: schemas("daemon_host_get"),
            handler: handle_daemon_host_get,
        },
        RegisteredController {
            schema: schemas("daemon_host_set"),
            handler: handle_daemon_host_set,
        },
    ]
}

/// Returns the specific `ControllerSchema` for a given service function.
///
/// # Arguments
///
/// * `function` - The name of the service function (e.g., "install", "restart").
pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "install" | "restart" | "start" | "stop" | "status" | "uninstall" => ControllerSchema {
            namespace: "service",
            function: match function {
                "install" => "install",
                "restart" => "restart",
                "start" => "start",
                "stop" => "stop",
                "status" => "status",
                _ => "uninstall",
            },
            description: "Manage desktop service lifecycle.",
            inputs: if function == "restart" {
                vec![
                    FieldSchema {
                        name: "source",
                        ty: TypeSchema::String,
                        comment: "Optional caller label for restart diagnostics.",
                        required: false,
                    },
                    FieldSchema {
                        name: "reason",
                        ty: TypeSchema::String,
                        comment: "Optional free-form reason for the restart request.",
                        required: false,
                    },
                ]
            } else {
                vec![]
            },
            outputs: vec![FieldSchema {
                name: "status",
                ty: if function == "restart" {
                    TypeSchema::Json
                } else {
                    TypeSchema::Ref("ServiceStatus")
                },
                comment: if function == "restart" {
                    "Restart request acknowledgement payload."
                } else {
                    "Service status payload."
                },
                required: true,
            }],
        },
        "daemon_host_get" => ControllerSchema {
            namespace: "service",
            function: "daemon_host_get",
            description: "Read daemon host UI preferences.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("DaemonHostConfig"),
                comment: "Daemon host preference payload.",
                required: true,
            }],
        },
        "daemon_host_set" => ControllerSchema {
            namespace: "service",
            function: "daemon_host_set",
            description: "Update daemon host UI preferences.",
            inputs: vec![FieldSchema {
                name: "show_tray",
                ty: TypeSchema::Bool,
                comment: "Show tray icon in daemon-host mode.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("DaemonHostConfig"),
                comment: "Updated daemon host preference payload.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "service",
            function: "unknown",
            description: "Unknown service controller function.",
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

fn handle_install(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_install(&config).await?)
    })
}

/// Service controller for `service.start`.
fn handle_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_start(&config).await?)
    })
}

#[derive(Debug, Deserialize)]
struct ServiceRestartParams {
    source: Option<String>,
    reason: Option<String>,
}

/// Service controller for `service.restart`.
///
/// Service restart is intentionally config-free.
///
/// Unlike install/start/stop/status, the restart action targets the currently
/// running core process itself, so it only needs restart metadata and not the
/// persisted service config.
fn handle_restart(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: ServiceRestartParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        to_json(
            crate::openhuman::service::rpc::service_restart(payload.source, payload.reason).await?,
        )
    })
}

/// Service controller for `service.stop`.
fn handle_stop(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_stop(&config).await?)
    })
}

/// Service controller for `service.status`.
fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_status(&config).await?)
    })
}

/// Service controller for `service.uninstall`.
fn handle_uninstall(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_uninstall(&config).await?)
    })
}

#[derive(Debug, Deserialize)]
struct DaemonHostSetParams {
    show_tray: bool,
}

/// Service controller for `service.daemon_host_get`.
fn handle_daemon_host_get(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::daemon_host_get(&config).await?)
    })
}

/// Service controller for `service.daemon_host_set`.
fn handle_daemon_host_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: DaemonHostSetParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::daemon_host_set(&config, payload.show_tray).await?)
    })
}

/// Formats the RpcOutcome as an OpenHuman-standard JSON result.
fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
