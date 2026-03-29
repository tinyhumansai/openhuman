use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

const SOCKET_UNAVAILABLE_MSG: &str =
    "native skill runtime and socket manager are not available in this build";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("connect"),
        schemas("disconnect"),
        schemas("state"),
        schemas("emit"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("connect"),
            handler: handle_socket_unavailable,
        },
        RegisteredController {
            schema: schemas("disconnect"),
            handler: handle_socket_unavailable,
        },
        RegisteredController {
            schema: schemas("state"),
            handler: handle_socket_unavailable,
        },
        RegisteredController {
            schema: schemas("emit"),
            handler: handle_socket_unavailable,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "connect" | "disconnect" | "state" | "emit" => ControllerSchema {
            namespace: "socket",
            function: match function {
                "connect" => "connect",
                "disconnect" => "disconnect",
                "state" => "state",
                _ => "emit",
            },
            description: "Skill runtime socket manager bridge.",
            inputs: vec![FieldSchema {
                name: "payload",
                ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                comment: "Socket request payload.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Socket response payload.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "socket",
            function: "unknown",
            description: "Unknown skills controller function.",
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

fn handle_socket_unavailable(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { Err(SOCKET_UNAVAILABLE_MSG.to_string()) })
}
