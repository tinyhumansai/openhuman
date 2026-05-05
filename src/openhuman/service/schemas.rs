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
        schemas("shutdown"),
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
            schema: schemas("shutdown"),
            handler: handle_shutdown,
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
        "install" | "restart" | "shutdown" | "start" | "stop" | "status" | "uninstall" => {
            let lifecycle_self_signal = function == "restart" || function == "shutdown";
            ControllerSchema {
                namespace: "service",
                function: match function {
                    "install" => "install",
                    "restart" => "restart",
                    "shutdown" => "shutdown",
                    "start" => "start",
                    "stop" => "stop",
                    "status" => "status",
                    _ => "uninstall",
                },
                description: "Manage desktop service lifecycle.",
                inputs: if lifecycle_self_signal {
                    vec![
                        FieldSchema {
                            name: "source",
                            ty: TypeSchema::String,
                            comment: "Optional caller label for diagnostics.",
                            required: false,
                        },
                        FieldSchema {
                            name: "reason",
                            ty: TypeSchema::String,
                            comment: "Optional free-form reason for the request.",
                            required: false,
                        },
                    ]
                } else {
                    vec![]
                },
                outputs: vec![FieldSchema {
                    name: "status",
                    ty: if lifecycle_self_signal {
                        TypeSchema::Json
                    } else {
                        TypeSchema::Ref("ServiceStatus")
                    },
                    comment: match function {
                        "restart" => "Restart request acknowledgement payload.",
                        "shutdown" => "Shutdown request acknowledgement payload.",
                        _ => "Service status payload.",
                    },
                    required: true,
                }],
            }
        }
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

/// Service controller for `service.shutdown`.
///
/// Uses the same `{source, reason}` shape as `service.restart`.
fn handle_shutdown(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: ServiceRestartParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        to_json(
            crate::openhuman::service::rpc::service_shutdown(payload.source, payload.reason)
                .await?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_nine() {
        assert_eq!(all_controller_schemas().len(), 9);
    }

    #[test]
    fn all_controllers_returns_nine() {
        assert_eq!(all_registered_controllers().len(), 9);
    }

    #[test]
    fn all_schemas_use_service_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "service");
            assert!(!s.description.is_empty());
        }
    }

    #[test]
    fn lifecycle_schemas_have_no_inputs_except_self_signals() {
        for fn_name in ["install", "start", "stop", "status", "uninstall"] {
            let s = schemas(fn_name);
            assert!(
                s.inputs.is_empty(),
                "{fn_name} should have no inputs, got {}",
                s.inputs.len()
            );
        }
    }

    #[test]
    fn restart_and_shutdown_schemas_have_optional_source_and_reason() {
        for fn_name in ["restart", "shutdown"] {
            let s = schemas(fn_name);
            assert_eq!(s.function, fn_name);
            assert_eq!(s.inputs.len(), 2, "{fn_name} should have 2 inputs");
            for input in &s.inputs {
                assert!(
                    !input.required,
                    "{fn_name} input '{}' should be optional",
                    input.name
                );
            }
        }
    }

    #[test]
    fn daemon_host_get_schema_has_no_inputs() {
        let s = schemas("daemon_host_get");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn daemon_host_set_requires_show_tray() {
        let s = schemas("daemon_host_set");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "show_tray");
        assert!(s.inputs[0].required);
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = schemas("nonexistent");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "service");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        assert_eq!(s.len(), c.len());
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, ctrl.schema.function);
        }
    }

    #[test]
    fn known_functions_resolve_correctly() {
        for fn_name in [
            "install",
            "restart",
            "shutdown",
            "start",
            "stop",
            "status",
            "uninstall",
            "daemon_host_get",
            "daemon_host_set",
        ] {
            let s = schemas(fn_name);
            assert_ne!(s.function, "unknown", "{fn_name} fell through");
        }
    }
}
