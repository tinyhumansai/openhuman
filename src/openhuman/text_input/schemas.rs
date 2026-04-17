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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_controller_schemas_returns_5() {
        assert_eq!(all_controller_schemas().len(), 5);
    }

    #[test]
    fn all_registered_controllers_returns_5() {
        assert_eq!(all_registered_controllers().len(), 5);
    }

    #[test]
    fn schemas_and_controllers_are_consistent() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        assert_eq!(s.len(), c.len());
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.namespace, ctrl.schema.namespace);
            assert_eq!(schema.function, ctrl.schema.function);
        }
    }

    #[test]
    fn all_schemas_use_text_input_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "text_input");
            assert!(!s.description.is_empty());
            assert!(!s.outputs.is_empty());
        }
    }

    #[test]
    fn read_field_schema() {
        let s = schemas("read_field");
        assert_eq!(s.function, "read_field");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "include_bounds");
        assert!(!s.inputs[0].required);
    }

    #[test]
    fn insert_text_schema() {
        let s = schemas("insert_text");
        assert_eq!(s.function, "insert_text");
        assert_eq!(s.inputs.len(), 4);
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["text"]);
    }

    #[test]
    fn show_ghost_schema() {
        let s = schemas("show_ghost");
        assert_eq!(s.function, "show_ghost");
        assert_eq!(s.inputs.len(), 3);
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["text"]);
    }

    #[test]
    fn dismiss_ghost_schema() {
        let s = schemas("dismiss_ghost");
        assert_eq!(s.function, "dismiss_ghost");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn accept_ghost_schema() {
        let s = schemas("accept_ghost");
        assert_eq!(s.function, "accept_ghost");
        assert_eq!(s.inputs.len(), 4);
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["text"]);
    }

    #[test]
    fn unknown_function_returns_fallback() {
        let s = schemas("nonexistent");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "text_input");
    }

    #[test]
    fn deserialize_params_valid() {
        let mut m = Map::new();
        m.insert("tunnel_uuid".into(), Value::String("x".into()));
        // Just test the generic helper works on a simple struct
        #[derive(serde::Deserialize)]
        struct Simple {
            tunnel_uuid: String,
        }
        let result = deserialize_params::<Simple>(m);
        assert!(result.is_ok());
    }

    #[test]
    fn deserialize_params_invalid() {
        let err =
            deserialize_params::<super::super::types::InsertTextParams>(Map::new()).unwrap_err();
        assert!(err.contains("invalid params"));
    }

    #[test]
    fn deserialize_params_or_default_empty_returns_default() {
        let result =
            deserialize_params_or_default::<super::super::types::ReadFieldParams>(Map::new());
        // Should be default value, not panic
        let _ = result;
    }

    #[test]
    fn deserialize_params_or_default_invalid_returns_default() {
        let mut m = Map::new();
        m.insert(
            "bad_field".into(),
            Value::Number(serde_json::Number::from(42)),
        );
        let result = deserialize_params_or_default::<super::super::types::ReadFieldParams>(m);
        let _ = result;
    }

    #[test]
    fn json_output_helper() {
        let f = json_output("result", "desc");
        assert_eq!(f.name, "result");
        assert!(f.required);
        assert!(matches!(f.ty, TypeSchema::Json));
    }

    #[test]
    fn to_json_helper() {
        let outcome = RpcOutcome::single_log(json!({"ok": true}), "log");
        let result = to_json(outcome);
        assert!(result.is_ok());
    }
}
