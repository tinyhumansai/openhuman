//! Controller schemas for the `guided_flows` domain.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

type SchemaBuilder = fn() -> ControllerSchema;
type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;

struct Def {
    function: &'static str,
    schema: SchemaBuilder,
    handler: ControllerHandler,
}

const DEFS: &[Def] = &[
    Def {
        function: "list_flows",
        schema: schema_list_flows,
        handler: handle_list_flows,
    },
    Def {
        function: "start_flow",
        schema: schema_start_flow,
        handler: handle_start_flow,
    },
    Def {
        function: "submit_answer",
        schema: schema_submit_answer,
        handler: handle_submit_answer,
    },
    Def {
        function: "get_session",
        schema: schema_get_session,
        handler: handle_get_session,
    },
    Def {
        function: "register_flow",
        schema: schema_register_flow,
        handler: handle_register_flow,
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
        .unwrap_or_else(schema_unknown)
}

fn schema_list_flows() -> ControllerSchema {
    ControllerSchema {
        namespace: "guided_flows",
        function: "list_flows",
        description: "List all available guided recommendation flows.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success flag.",
                required: true,
            },
            FieldSchema {
                name: "flows",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of flow summaries.",
                required: true,
            },
        ],
    }
}

fn schema_start_flow() -> ControllerSchema {
    ControllerSchema {
        namespace: "guided_flows",
        function: "start_flow",
        description: "Start a new guided flow session. Returns the first step prompt.",
        inputs: vec![
            FieldSchema {
                name: "flow_id",
                ty: TypeSchema::String,
                comment: "Flow definition ID.",
                required: true,
            },
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Optional session UUID.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success flag.",
                required: true,
            },
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Session key.",
                required: true,
            },
            FieldSchema {
                name: "flow_id",
                ty: TypeSchema::String,
                comment: "Flow ID.",
                required: true,
            },
            FieldSchema {
                name: "current_step",
                ty: TypeSchema::String,
                comment: "Current step ID.",
                required: true,
            },
            FieldSchema {
                name: "state",
                ty: TypeSchema::String,
                comment: "Session state.",
                required: true,
            },
        ],
    }
}

fn schema_submit_answer() -> ControllerSchema {
    ControllerSchema {
        namespace: "guided_flows",
        function: "submit_answer",
        description: "Submit an answer for the current step and advance the flow.",
        inputs: vec![
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Session key.",
                required: true,
            },
            FieldSchema {
                name: "step_id",
                ty: TypeSchema::String,
                comment: "Step being answered.",
                required: true,
            },
            FieldSchema {
                name: "value",
                ty: TypeSchema::Json,
                comment: "Answer value (string, bool, number, or array).",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success flag.",
                required: true,
            },
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Session key.",
                required: true,
            },
            FieldSchema {
                name: "state",
                ty: TypeSchema::String,
                comment: "Session state after answer.",
                required: true,
            },
            FieldSchema {
                name: "current_step",
                ty: TypeSchema::String,
                comment: "Next step ID (if active).",
                required: true,
            },
            FieldSchema {
                name: "recommendation",
                ty: TypeSchema::Json,
                comment: "Recommendation (if completed).",
                required: false,
            },
        ],
    }
}

fn schema_get_session() -> ControllerSchema {
    ControllerSchema {
        namespace: "guided_flows",
        function: "get_session",
        description: "Get the current state of a guided flow session.",
        inputs: vec![FieldSchema {
            name: "session_id",
            ty: TypeSchema::String,
            comment: "Session key.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success flag.",
                required: true,
            },
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Session key.",
                required: true,
            },
            FieldSchema {
                name: "flow_id",
                ty: TypeSchema::String,
                comment: "Flow ID.",
                required: true,
            },
            FieldSchema {
                name: "state",
                ty: TypeSchema::String,
                comment: "Session state.",
                required: true,
            },
            FieldSchema {
                name: "current_step",
                ty: TypeSchema::String,
                comment: "Current step.",
                required: true,
            },
            FieldSchema {
                name: "answers_count",
                ty: TypeSchema::F64,
                comment: "Number of answers submitted.",
                required: true,
            },
        ],
    }
}

fn schema_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "guided_flows",
        function: "unknown",
        description: "Unknown guided_flows function.",
        inputs: vec![FieldSchema {
            name: "function",
            ty: TypeSchema::String,
            comment: "Requested function.",
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

fn handle_list_flows(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::rpc::handle_list_flows(p)
            .await?
            .into_cli_compatible_json()
    })
}
fn handle_start_flow(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::rpc::handle_start_flow(p)
            .await?
            .into_cli_compatible_json()
    })
}
fn handle_submit_answer(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::rpc::handle_submit_answer(p)
            .await?
            .into_cli_compatible_json()
    })
}
fn handle_get_session(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::rpc::handle_get_session(p)
            .await?
            .into_cli_compatible_json()
    })
}
fn handle_register_flow(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::rpc::handle_register_flow(p)
            .await?
            .into_cli_compatible_json()
    })
}

fn schema_register_flow() -> ControllerSchema {
    ControllerSchema {
        namespace: "guided_flows",
        function: "register_flow",
        description: "Register a custom flow definition.",
        inputs: vec![
            FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Unique flow ID.",
                required: true,
            },
            FieldSchema {
                name: "name",
                ty: TypeSchema::String,
                comment: "Display name.",
                required: true,
            },
            FieldSchema {
                name: "description",
                ty: TypeSchema::String,
                comment: "Flow description.",
                required: true,
            },
            FieldSchema {
                name: "start_step",
                ty: TypeSchema::String,
                comment: "ID of the first step.",
                required: true,
            },
            FieldSchema {
                name: "steps",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of step definitions.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "flow_id",
                ty: TypeSchema::String,
                comment: "Registered flow ID.",
                required: true,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_handlers_match_schemas() {
        let s: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        let h: Vec<_> = all_registered_controllers()
            .into_iter()
            .map(|c| c.schema.function)
            .collect();
        assert_eq!(s, h);
        assert_eq!(
            s,
            vec![
                "list_flows",
                "start_flow",
                "submit_answer",
                "get_session",
                "register_flow"
            ]
        );
    }

    #[test]
    fn lookup_unknown() {
        assert_eq!(schemas("nope").function, "unknown");
    }

    #[test]
    fn all_schemas_have_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "guided_flows");
        }
    }
}
