use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::screen_intelligence::{
    InputActionParams, PermissionRequestParams, StartSessionParams, StopSessionParams,
};
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct AccessibilityVisionRecentParams {
    limit: Option<usize>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("status"),
        schemas("request_permissions"),
        schemas("request_permission"),
        schemas("refresh_permissions"),
        schemas("start_session"),
        schemas("stop_session"),
        schemas("capture_now"),
        schemas("capture_image_ref"),
        schemas("input_action"),
        schemas("vision_recent"),
        schemas("vision_flush"),
        schemas("capture_test"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("request_permissions"),
            handler: handle_request_permissions,
        },
        RegisteredController {
            schema: schemas("request_permission"),
            handler: handle_request_permission,
        },
        RegisteredController {
            schema: schemas("refresh_permissions"),
            handler: handle_refresh_permissions,
        },
        RegisteredController {
            schema: schemas("start_session"),
            handler: handle_start_session,
        },
        RegisteredController {
            schema: schemas("stop_session"),
            handler: handle_stop_session,
        },
        RegisteredController {
            schema: schemas("capture_now"),
            handler: handle_capture_now,
        },
        RegisteredController {
            schema: schemas("capture_image_ref"),
            handler: handle_capture_image_ref,
        },
        RegisteredController {
            schema: schemas("input_action"),
            handler: handle_input_action,
        },
        RegisteredController {
            schema: schemas("vision_recent"),
            handler: handle_vision_recent,
        },
        RegisteredController {
            schema: schemas("vision_flush"),
            handler: handle_vision_flush,
        },
        RegisteredController {
            schema: schemas("capture_test"),
            handler: handle_capture_test,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "status",
            description: "Read screen intelligence accessibility status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Accessibility status payload.")],
        },
        "request_permissions" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "request_permissions",
            description: "Request required accessibility permissions.",
            inputs: vec![],
            outputs: vec![json_output("permissions", "Permission status payload.")],
        },
        "request_permission" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "request_permission",
            description: "Request one accessibility permission.",
            inputs: vec![FieldSchema {
                name: "permission",
                ty: TypeSchema::String,
                comment: "Permission name.",
                required: true,
            }],
            outputs: vec![json_output("permissions", "Permission status payload.")],
        },
        "refresh_permissions" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "refresh_permissions",
            description: "Re-detect current macOS permission state without requesting new grants. \
                           Call this after the sidecar restarts to read freshly granted permissions.",
            inputs: vec![],
            outputs: vec![json_output("permissions", "Freshly detected permission status.")],
        },
        "start_session" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "start_session",
            description: "Start screen intelligence session.",
            inputs: vec![
                FieldSchema {
                    name: "sample_interval_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Capture interval in milliseconds.",
                    required: false,
                },
                FieldSchema {
                    name: "capture_policy",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Capture policy mode.",
                    required: false,
                },
            ],
            outputs: vec![json_output("session", "Session status payload.")],
        },
        "stop_session" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "stop_session",
            description: "Stop screen intelligence session.",
            inputs: vec![FieldSchema {
                name: "reason",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional stop reason.",
                required: false,
            }],
            outputs: vec![json_output("session", "Session status payload.")],
        },
        "capture_now" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "capture_now",
            description: "Trigger immediate screen capture.",
            inputs: vec![],
            outputs: vec![json_output("capture", "Capture result payload.")],
        },
        "capture_image_ref" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "capture_image_ref",
            description: "Capture screenshot and return image ref.",
            inputs: vec![],
            outputs: vec![json_output("capture", "Capture image_ref payload.")],
        },
        "input_action" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "input_action",
            description: "Perform input action through accessibility automation.",
            inputs: vec![FieldSchema {
                name: "action",
                ty: TypeSchema::Ref("InputActionParams"),
                comment: "Input action payload.",
                required: true,
            }],
            outputs: vec![json_output("result", "Input action result payload.")],
        },
        "vision_recent" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "vision_recent",
            description: "Read recent vision summaries.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of summaries.",
                required: false,
            }],
            outputs: vec![json_output("result", "Vision recent payload.")],
        },
        "vision_flush" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "vision_flush",
            description: "Flush stored vision summaries.",
            inputs: vec![],
            outputs: vec![json_output("result", "Vision flush payload.")],
        },
        "capture_test" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "capture_test",
            description: "Standalone capture test with diagnostics (no session required).",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "Capture test result with diagnostics.",
            )],
        },
        _ => ControllerSchema {
            namespace: "screen_intelligence",
            function: "unknown",
            description: "Unknown screen_intelligence controller function.",
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

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(crate::openhuman::screen_intelligence::rpc::accessibility_status().await?)
    })
}

fn handle_request_permissions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_request_permissions().await?,
        )
    })
}

fn handle_refresh_permissions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_refresh_permissions().await?,
        )
    })
}

fn handle_request_permission(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<PermissionRequestParams>(params)?;
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_request_permission(payload)
                .await?,
        )
    })
}

fn handle_start_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<StartSessionParams>(params)?;
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_start_session(payload)
                .await?,
        )
    })
}

fn handle_stop_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<StopSessionParams>(params)?;
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_stop_session(payload).await?,
        )
    })
}

fn handle_capture_now(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(crate::openhuman::screen_intelligence::rpc::accessibility_capture_now().await?)
    })
}

fn handle_capture_image_ref(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_capture_image_ref().await?,
        )
    })
}

fn handle_input_action(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<InputActionParams>(params)?;
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_input_action(payload).await?,
        )
    })
}

fn handle_vision_recent(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<AccessibilityVisionRecentParams>(params)?;
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_vision_recent(payload.limit)
                .await?,
        )
    })
}

fn handle_vision_flush(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(crate::openhuman::screen_intelligence::rpc::accessibility_vision_flush().await?)
    })
}

fn handle_capture_test(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(crate::openhuman::screen_intelligence::rpc::accessibility_capture_test().await?)
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
