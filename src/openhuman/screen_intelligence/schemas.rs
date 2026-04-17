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
        schemas("globe_listener_start"),
        schemas("globe_listener_poll"),
        schemas("globe_listener_stop"),
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
        RegisteredController {
            schema: schemas("globe_listener_start"),
            handler: handle_globe_listener_start,
        },
        RegisteredController {
            schema: schemas("globe_listener_poll"),
            handler: handle_globe_listener_poll,
        },
        RegisteredController {
            schema: schemas("globe_listener_stop"),
            handler: handle_globe_listener_stop,
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
            description:
                "Request accessibility automation permissions without prompting for screen recording.",
            inputs: vec![],
            outputs: vec![json_output("permissions", "Permission status payload.")],
        },
        "request_permission" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "request_permission",
            description: "Request one permission such as accessibility, input monitoring, or screen recording.",
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
                    name: "consent",
                    ty: TypeSchema::Bool,
                    comment: "Explicit user consent to start the accessibility session.",
                    required: true,
                },
                FieldSchema {
                    name: "ttl_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Session time-to-live in seconds.",
                    required: false,
                },
                FieldSchema {
                    name: "screen_monitoring",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Whether screen recording capture should be enabled for this session.",
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
        "globe_listener_start" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "globe_listener_start",
            description: "Start the macOS Globe/Fn hotkey listener helper.",
            inputs: vec![],
            outputs: vec![json_output("result", "Globe hotkey listener status.")],
        },
        "globe_listener_poll" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "globe_listener_poll",
            description: "Drain pending Globe/Fn listener events and return listener status.",
            inputs: vec![],
            outputs: vec![json_output("result", "Globe hotkey listener poll result.")],
        },
        "globe_listener_stop" => ControllerSchema {
            namespace: "screen_intelligence",
            function: "globe_listener_stop",
            description: "Stop the macOS Globe/Fn hotkey listener helper.",
            inputs: vec![],
            outputs: vec![json_output("result", "Globe hotkey listener status.")],
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

fn handle_globe_listener_start(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_globe_listener_start()
                .await?,
        )
    })
}

fn handle_globe_listener_poll(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_globe_listener_poll().await?,
        )
    })
}

fn handle_globe_listener_stop(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(
            crate::openhuman::screen_intelligence::rpc::accessibility_globe_listener_stop().await?,
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_counts_match_and_nonempty() {
        let s = all_controller_schemas();
        let h = all_registered_controllers();
        assert_eq!(s.len(), h.len());
        assert!(s.len() >= 10);
    }

    #[test]
    fn all_schemas_use_accessibility_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(
                s.namespace, "screen_intelligence",
                "function {}",
                s.function
            );
            assert!(!s.description.is_empty());
            assert!(!s.outputs.is_empty());
        }
    }

    #[test]
    fn unknown_function_returns_unknown_schema() {
        let s = schemas("no_such_fn");
        assert_eq!(s.function, "unknown");
    }

    #[test]
    fn every_known_key_resolves_to_non_unknown() {
        let keys = [
            "status",
            "request_permissions",
            "request_permission",
            "refresh_permissions",
            "start_session",
            "stop_session",
            "capture_now",
            "capture_image_ref",
            "input_action",
            "vision_recent",
            "vision_flush",
            "capture_test",
            "globe_listener_start",
            "globe_listener_poll",
            "globe_listener_stop",
        ];
        for k in keys {
            let s = schemas(k);
            assert_eq!(s.namespace, "screen_intelligence");
            assert_ne!(s.function, "unknown", "key `{k}` fell through");
        }
    }

    #[test]
    fn registered_controllers_use_accessibility_namespace() {
        for h in all_registered_controllers() {
            assert_eq!(h.schema.namespace, "screen_intelligence");
            assert!(!h.schema.function.is_empty());
        }
    }

    #[test]
    fn status_schema_has_no_inputs() {
        let s = schemas("status");
        assert!(s.inputs.is_empty());
        assert_eq!(s.outputs.len(), 1);
    }

    #[test]
    fn request_permissions_schema_has_no_inputs() {
        assert!(schemas("request_permissions").inputs.is_empty());
    }

    #[test]
    fn request_permission_requires_permission_input() {
        let s = schemas("request_permission");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "permission");
        assert!(s.inputs[0].required);
    }

    #[test]
    fn refresh_permissions_schema_has_no_inputs() {
        assert!(schemas("refresh_permissions").inputs.is_empty());
    }

    #[test]
    fn start_session_schema_requires_consent() {
        let s = schemas("start_session");
        let consent = s.inputs.iter().find(|f| f.name == "consent").unwrap();
        assert!(consent.required);
    }

    #[test]
    fn stop_session_schema_has_optional_reason() {
        let s = schemas("stop_session");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "reason");
        assert!(!s.inputs[0].required);
    }

    #[test]
    fn capture_now_schema_has_optional_inputs() {
        let s = schemas("capture_now");
        for input in &s.inputs {
            assert!(
                !input.required,
                "capture_now input '{}' should be optional",
                input.name
            );
        }
    }

    #[test]
    fn capture_image_ref_schema_has_no_inputs() {
        let s = schemas("capture_image_ref");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn input_action_schema_requires_action() {
        let s = schemas("input_action");
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert!(required.contains(&"action"));
    }

    #[test]
    fn vision_recent_schema() {
        let s = schemas("vision_recent");
        assert!(!s.description.is_empty());
    }

    #[test]
    fn vision_flush_schema_has_no_inputs() {
        assert!(schemas("vision_flush").inputs.is_empty());
    }

    #[test]
    fn capture_test_schema() {
        let s = schemas("capture_test");
        assert_eq!(s.function, "capture_test");
    }

    #[test]
    fn globe_listener_start_schema() {
        let s = schemas("globe_listener_start");
        assert_eq!(s.function, "globe_listener_start");
    }

    #[test]
    fn globe_listener_poll_schema() {
        let s = schemas("globe_listener_poll");
        assert_eq!(s.function, "globe_listener_poll");
    }

    #[test]
    fn globe_listener_stop_schema() {
        let s = schemas("globe_listener_stop");
        assert_eq!(s.function, "globe_listener_stop");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, ctrl.schema.function);
        }
    }
}
