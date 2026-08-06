use super::ops;
use super::types::{MonitorReadRequest, MonitorStartRequest, MonitorStopRequest};
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

pub fn all_monitor_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("start"),
        schemas("list"),
        schemas("stop"),
        schemas("read"),
    ]
}

pub fn all_monitor_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("start"),
            handler: handle_start,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("stop"),
            handler: handle_stop,
        },
        RegisteredController {
            schema: schemas("read"),
            handler: handle_read,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "start" => ControllerSchema {
            namespace: "monitor",
            function: "start",
            description: "Start a bounded background command monitor.",
            inputs: vec![
                field("command", TypeSchema::String, "Shell command to monitor."),
                optional(
                    "description",
                    TypeSchema::String,
                    "Human-readable monitor label.",
                ),
                optional(
                    "timeout_ms",
                    TypeSchema::U64,
                    "Maximum runtime in milliseconds.",
                ),
                optional(
                    "persistent",
                    TypeSchema::Bool,
                    "Whether the monitor is intended to survive a short-lived turn.",
                ),
                optional(
                    "category",
                    TypeSchema::String,
                    "Escalate-only shell risk category hint.",
                ),
            ],
            outputs: vec![json_output("result", "Started monitor envelope.")],
        },
        "list" => ControllerSchema {
            namespace: "monitor",
            function: "list",
            description: "List background command monitors.",
            inputs: vec![],
            outputs: vec![json_output("result", "Monitor snapshots.")],
        },
        "stop" => ControllerSchema {
            namespace: "monitor",
            function: "stop",
            description: "Stop a running background command monitor.",
            inputs: vec![field("monitor_id", TypeSchema::String, "Monitor id.")],
            outputs: vec![json_output("result", "Stopped monitor envelope.")],
        },
        "read" => ControllerSchema {
            namespace: "monitor",
            function: "read",
            description: "Read bounded monitor output.",
            inputs: vec![
                field("monitor_id", TypeSchema::String, "Monitor id."),
                optional(
                    "max_bytes",
                    TypeSchema::U64,
                    "Maximum bytes to read from the output tail.",
                ),
            ],
            outputs: vec![json_output("result", "Output tail envelope.")],
        },
        _ => ControllerSchema {
            namespace: "monitor",
            function: "unknown",
            description: "Unknown monitor controller.",
            inputs: vec![],
            outputs: vec![json_output("error", "Error details.")],
        },
    }
}

fn handle_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: MonitorStartRequest = parse_params(params)?;
        ops::start_default(payload)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { ops::list().await?.into_cli_compatible_json() })
}

fn handle_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: MonitorStopRequest = parse_params(params)?;
        ops::stop(payload).await?.into_cli_compatible_json()
    })
}

fn handle_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: MonitorReadRequest = parse_params(params)?;
        ops::read(payload).await?.into_cli_compatible_json()
    })
}

fn parse_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

fn optional(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(ty)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_declares_four_controllers() {
        assert_eq!(all_monitor_controller_schemas().len(), 4);
        assert_eq!(all_monitor_registered_controllers().len(), 4);
    }

    #[test]
    fn start_schema_requires_command() {
        let schema = schemas("start");
        assert!(schema
            .inputs
            .iter()
            .any(|f| f.name == "command" && f.required));
        assert_eq!(schema.namespace, "monitor");
    }
}
