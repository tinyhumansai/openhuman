//! Controller schema definitions and handler registration for `text_input`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// Public registry API
// ---------------------------------------------------------------------------

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("read_field"),
        schemas("insert_text"),
        schemas("show_ghost"),
        schemas("dismiss_ghost"),
        schemas("accept_ghost"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("read_field"),
            handler: handle_read_field,
        },
        RegisteredController {
            schema: schemas("insert_text"),
            handler: handle_insert_text,
        },
        RegisteredController {
            schema: schemas("show_ghost"),
            handler: handle_show_ghost,
        },
        RegisteredController {
            schema: schemas("dismiss_ghost"),
            handler: handle_dismiss_ghost,
        },
        RegisteredController {
            schema: schemas("accept_ghost"),
            handler: handle_accept_ghost,
        },
    ]
}

// ---------------------------------------------------------------------------
// Schema definitions
// ---------------------------------------------------------------------------

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "read_field" => ControllerSchema {
            namespace: "text_input",
            function: "read_field",
            description: "Read the currently focused text input field contents.",
            inputs: vec![FieldSchema {
                name: "include_bounds",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "If true, include element bounds in the response.",
                required: false,
            }],
            outputs: vec![json_output("field", "Focused text field context.")],
        },
        "insert_text" => ControllerSchema {
            namespace: "text_input",
            function: "insert_text",
            description: "Insert text into the currently focused input field.",
            inputs: vec![
                FieldSchema {
                    name: "text",
                    ty: TypeSchema::String,
                    comment: "Text to insert.",
                    required: true,
                },
                FieldSchema {
                    name: "validate_focus",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "If true, validate focus hasn't shifted before inserting.",
                    required: false,
                },
                FieldSchema {
                    name: "expected_app",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Expected app name for focus validation.",
                    required: false,
                },
                FieldSchema {
                    name: "expected_role",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Expected element role for focus validation.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Insert operation result.")],
        },
        "show_ghost" => ControllerSchema {
            namespace: "text_input",
            function: "show_ghost",
            description: "Show ghost text overlay near the focused input field.",
            inputs: vec![
                FieldSchema {
                    name: "text",
                    ty: TypeSchema::String,
                    comment: "Ghost text to display.",
                    required: true,
                },
                FieldSchema {
                    name: "ttl_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Auto-dismiss after N milliseconds (default: 3000).",
                    required: false,
                },
                FieldSchema {
                    name: "bounds",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Position overlay near these bounds {x,y,width,height}. If omitted, reads from focused field.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Ghost text display result.")],
        },
        "dismiss_ghost" => ControllerSchema {
            namespace: "text_input",
            function: "dismiss_ghost",
            description: "Dismiss the ghost text overlay.",
            inputs: vec![],
            outputs: vec![json_output("result", "Dismiss result.")],
        },
        "accept_ghost" => ControllerSchema {
            namespace: "text_input",
            function: "accept_ghost",
            description:
                "Dismiss ghost text and insert the accepted text atomically.",
            inputs: vec![
                FieldSchema {
                    name: "text",
                    ty: TypeSchema::String,
                    comment: "Text to insert (the accepted ghost suggestion).",
                    required: true,
                },
                FieldSchema {
                    name: "validate_focus",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "If true, validate focus hasn't shifted before inserting.",
                    required: false,
                },
                FieldSchema {
                    name: "expected_app",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Expected app name for focus validation.",
                    required: false,
                },
                FieldSchema {
                    name: "expected_role",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Expected element role for focus validation.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Accept + insert result.")],
        },
        _ => ControllerSchema {
            namespace: "text_input",
            function: "unknown",
            description: "Unknown text_input controller function.",
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

fn handle_read_field(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params_or_default::<super::types::ReadFieldParams>(params);
        to_json(super::ops::read_field(payload).await?)
    })
}

fn handle_insert_text(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<super::types::InsertTextParams>(params)?;
        to_json(super::ops::insert_text(payload).await?)
    })
}

fn handle_show_ghost(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<super::types::ShowGhostTextParams>(params)?;
        to_json(super::ops::show_ghost(payload).await?)
    })
}

fn handle_dismiss_ghost(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(super::ops::dismiss_ghost().await?) })
}

fn handle_accept_ghost(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<super::types::AcceptGhostTextParams>(params)?;
        to_json(super::ops::accept_ghost(payload).await?)
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn deserialize_params_or_default<T: DeserializeOwned + Default>(params: Map<String, Value>) -> T {
    if params.is_empty() {
        T::default()
    } else {
        serde_json::from_value(Value::Object(params)).unwrap_or_default()
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
