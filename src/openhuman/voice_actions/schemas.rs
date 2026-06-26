//! Controller schemas for the `voice_actions` domain.

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use serde_json::{Map, Value};

type SchemaBuilder = fn() -> ControllerSchema;
type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;
struct Def {
    function: &'static str,
    schema: SchemaBuilder,
    handler: ControllerHandler,
}

const DEFS: &[Def] = &[
    Def {
        function: "recognize",
        schema: s_recognize,
        handler: h_recognize,
    },
    Def {
        function: "confirm",
        schema: s_confirm,
        handler: h_confirm,
    },
    Def {
        function: "reject",
        schema: s_reject,
        handler: h_reject,
    },
    Def {
        function: "get_intent",
        schema: s_get,
        handler: h_get,
    },
    Def {
        function: "list_mappings",
        schema: s_list,
        handler: h_list,
    },
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    DEFS.iter().map(|d| (d.schema)()).collect()
}
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    DEFS.iter()
        .map(|d| RegisteredController {
            schema: (d.schema)(),
            handler: d.handler,
        })
        .collect()
}
pub fn schemas(function: &str) -> ControllerSchema {
    DEFS.iter()
        .find(|d| d.function == function)
        .map(|d| (d.schema)())
        .unwrap_or_else(s_unknown)
}

fn s_recognize() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_actions",
        function: "recognize",
        description: "Recognize a voice intent from an utterance and map to a controller action.",
        inputs: vec![FieldSchema {
            name: "utterance",
            ty: TypeSchema::String,
            comment: "Spoken text.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "intent_id",
                ty: TypeSchema::String,
                comment: "Intent ID.",
                required: true,
            },
            FieldSchema {
                name: "action",
                ty: TypeSchema::String,
                comment: "Matched action.",
                required: true,
            },
            FieldSchema {
                name: "safety",
                ty: TypeSchema::String,
                comment: "safe|requires_confirmation|destructive.",
                required: true,
            },
            FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Intent status.",
                required: true,
            },
        ],
    }
}

fn s_confirm() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_actions",
        function: "confirm",
        description: "Confirm a pending voice action intent for execution.",
        inputs: vec![FieldSchema {
            name: "intent_id",
            ty: TypeSchema::String,
            comment: "Intent ID.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "intent_id",
                ty: TypeSchema::String,
                comment: "Intent ID.",
                required: true,
            },
            FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "New status.",
                required: true,
            },
        ],
    }
}

fn s_reject() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_actions",
        function: "reject",
        description: "Reject a pending voice action intent.",
        inputs: vec![FieldSchema {
            name: "intent_id",
            ty: TypeSchema::String,
            comment: "Intent ID.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "intent_id",
                ty: TypeSchema::String,
                comment: "Intent ID.",
                required: true,
            },
            FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "New status.",
                required: true,
            },
        ],
    }
}

fn s_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_actions",
        function: "get_intent",
        description: "Get voice intent details by ID.",
        inputs: vec![FieldSchema {
            name: "intent_id",
            ty: TypeSchema::String,
            comment: "Intent ID.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "intent_id",
                ty: TypeSchema::String,
                comment: "ID.",
                required: true,
            },
            FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Status.",
                required: true,
            },
            FieldSchema {
                name: "action",
                ty: TypeSchema::String,
                comment: "Action.",
                required: true,
            },
        ],
    }
}

fn s_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_actions",
        function: "list_mappings",
        description: "List all registered voice action mappings.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "mappings",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Action mappings.",
                required: true,
            },
        ],
    }
}

fn s_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_actions",
        function: "unknown",
        description: "Unknown voice_actions function.",
        inputs: vec![FieldSchema {
            name: "function",
            ty: TypeSchema::String,
            comment: "Requested.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Error.",
            required: true,
        }],
    }
}

fn h_recognize(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_recognize(p).await })
}
fn h_confirm(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_confirm(p).await })
}
fn h_reject(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_reject(p).await })
}
fn h_get(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_get_intent(p).await })
}
fn h_list(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_list_mappings(p).await })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handlers_match() {
        let s: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        let h: Vec<_> = all_registered_controllers()
            .into_iter()
            .map(|c| c.schema.function)
            .collect();
        assert_eq!(s, h);
        assert_eq!(s.len(), 5);
    }
    #[test]
    fn namespace_correct() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "voice_actions");
        }
    }
    #[test]
    fn unknown_lookup() {
        assert_eq!(schemas("nope").function, "unknown");
    }
}
