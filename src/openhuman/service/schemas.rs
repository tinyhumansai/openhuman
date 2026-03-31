use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("install"),
        schemas("start"),
        schemas("stop"),
        schemas("status"),
        schemas("uninstall"),
        schemas("daemon_host_get"),
        schemas("daemon_host_set"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("install"),
            handler: handle_install,
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

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "install" | "start" | "stop" | "status" | "uninstall" => ControllerSchema {
            namespace: "service",
            function: match function {
                "install" => "install",
                "start" => "start",
                "stop" => "stop",
                "status" => "status",
                _ => "uninstall",
            },
            description: "Manage desktop service lifecycle.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("ServiceStatus"),
                comment: "Service status payload.",
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

fn handle_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_start(&config).await?)
    })
}

fn handle_stop(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_stop(&config).await?)
    })
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::service_status(&config).await?)
    })
}

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

fn handle_daemon_host_get(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::daemon_host_get(&config).await?)
    })
}

fn handle_daemon_host_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: DaemonHostSetParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::service::rpc::daemon_host_set(&config, payload.show_tray).await?)
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
