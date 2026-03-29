use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::autocomplete::{
    AutocompleteAcceptParams, AutocompleteCurrentParams, AutocompleteSetStyleParams,
    AutocompleteStartParams, AutocompleteStopParams,
};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("status"),
        schemas("start"),
        schemas("stop"),
        schemas("current"),
        schemas("debug_focus"),
        schemas("accept"),
        schemas("set_style"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
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
            schema: schemas("current"),
            handler: handle_current,
        },
        RegisteredController {
            schema: schemas("debug_focus"),
            handler: handle_debug_focus,
        },
        RegisteredController {
            schema: schemas("accept"),
            handler: handle_accept,
        },
        RegisteredController {
            schema: schemas("set_style"),
            handler: handle_set_style,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "autocomplete",
            function: "status",
            description: "Read autocomplete engine status and latest suggestion metadata.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("AutocompleteStatus"),
                comment: "Current runtime status payload.",
                required: true,
            }],
        },
        "start" => ControllerSchema {
            namespace: "autocomplete",
            function: "start",
            description: "Start autocomplete engine with optional debounce override.",
            inputs: vec![FieldSchema {
                name: "debounce_ms",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Optional debounce interval in milliseconds.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("AutocompleteStartResult"),
                comment: "Whether the engine started.",
                required: true,
            }],
        },
        "stop" => ControllerSchema {
            namespace: "autocomplete",
            function: "stop",
            description: "Stop autocomplete engine and optionally record stop reason.",
            inputs: vec![FieldSchema {
                name: "reason",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional reason for stopping.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("AutocompleteStopResult"),
                comment: "Whether the engine stopped.",
                required: true,
            }],
        },
        "current" => ControllerSchema {
            namespace: "autocomplete",
            function: "current",
            description: "Compute current suggestion for provided or captured context.",
            inputs: vec![FieldSchema {
                name: "context",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional explicit context to score suggestions against.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("AutocompleteCurrentResult"),
                comment: "Current suggestion payload.",
                required: true,
            }],
        },
        "debug_focus" => ControllerSchema {
            namespace: "autocomplete",
            function: "debug_focus",
            description: "Inspect focused element and text context used by autocomplete.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("AutocompleteDebugFocusResult"),
                comment: "Focused context diagnostics.",
                required: true,
            }],
        },
        "accept" => ControllerSchema {
            namespace: "autocomplete",
            function: "accept",
            description: "Accept and apply current or provided autocomplete suggestion.",
            inputs: vec![FieldSchema {
                name: "suggestion",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional explicit suggestion value to apply.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("AutocompleteAcceptResult"),
                comment: "Suggestion acceptance result.",
                required: true,
            }],
        },
        "set_style" => ControllerSchema {
            namespace: "autocomplete",
            function: "set_style",
            description: "Update autocomplete style configuration fields.",
            inputs: vec![
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Enable or disable autocomplete.",
                    required: false,
                },
                FieldSchema {
                    name: "debounce_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Debounce interval override in milliseconds.",
                    required: false,
                },
                FieldSchema {
                    name: "max_chars",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum suggestion length in characters.",
                    required: false,
                },
                FieldSchema {
                    name: "style_preset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Named style preset.",
                    required: false,
                },
                FieldSchema {
                    name: "style_instructions",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Custom style instructions.",
                    required: false,
                },
                FieldSchema {
                    name: "style_examples",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Style examples used for prompt shaping.",
                    required: false,
                },
                FieldSchema {
                    name: "disabled_apps",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "App allow/deny override list.",
                    required: false,
                },
                FieldSchema {
                    name: "accept_with_tab",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Whether tab key applies suggestion.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("AutocompleteSetStyleResult"),
                comment: "Updated autocomplete style config.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "autocomplete",
            function: "unknown",
            description: "Unknown autocomplete controller function.",
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
    Box::pin(async { to_json(crate::openhuman::autocomplete::rpc::autocomplete_status().await?) })
}

fn handle_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<AutocompleteStartParams>(params)?;
        to_json(crate::openhuman::autocomplete::rpc::autocomplete_start(payload).await?)
    })
}

fn handle_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = if params.is_empty() {
            None
        } else {
            Some(deserialize_params::<AutocompleteStopParams>(params)?)
        };
        to_json(crate::openhuman::autocomplete::rpc::autocomplete_stop(payload).await?)
    })
}

fn handle_current(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = if params.is_empty() {
            None
        } else {
            Some(deserialize_params::<AutocompleteCurrentParams>(params)?)
        };
        to_json(crate::openhuman::autocomplete::rpc::autocomplete_current(payload).await?)
    })
}

fn handle_debug_focus(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        to_json(crate::openhuman::autocomplete::rpc::autocomplete_debug_focus().await?)
    })
}

fn handle_accept(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<AutocompleteAcceptParams>(params)?;
        to_json(crate::openhuman::autocomplete::rpc::autocomplete_accept(payload).await?)
    })
}

fn handle_set_style(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<AutocompleteSetStyleParams>(params)?;
        to_json(crate::openhuman::autocomplete::rpc::autocomplete_set_style(payload).await?)
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
