//! Controller schema definitions and registered handlers for the `meet`
//! domain.
//!
//! Mirrors the pattern used by `src/openhuman/notifications/schemas.rs`.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

type SchemaBuilder = fn() -> ControllerSchema;
type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;

struct MeetControllerDef {
    function: &'static str,
    schema: SchemaBuilder,
    handler: ControllerHandler,
}

const MEET_CONTROLLER_DEFS: &[MeetControllerDef] = &[MeetControllerDef {
    function: "join_call",
    schema: schema_join_call,
    handler: handle_join_call_wrap,
}];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    MEET_CONTROLLER_DEFS
        .iter()
        .map(|def| (def.schema)())
        .collect()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    MEET_CONTROLLER_DEFS
        .iter()
        .map(|def| RegisteredController {
            schema: (def.schema)(),
            handler: def.handler,
        })
        .collect()
}

pub fn schemas(function: &str) -> ControllerSchema {
    if let Some(def) = MEET_CONTROLLER_DEFS
        .iter()
        .find(|def| def.function == function)
    {
        return (def.schema)();
    }
    schema_unknown()
}

fn schema_join_call() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet",
        function: "join_call",
        description: "Validate a Google Meet URL and mint a request_id so the desktop \
                          shell can open a dedicated CEF webview that joins the call as an \
                          anonymous guest. Returns immediately — actual webview lifecycle \
                          is handled by the Tauri shell, not the core.",
        inputs: vec![
            FieldSchema {
                name: "meet_url",
                ty: TypeSchema::String,
                comment: "Full https://meet.google.com/<code> or /lookup/<id> URL.",
                required: true,
            },
            FieldSchema {
                name: "display_name",
                ty: TypeSchema::String,
                comment: "Display name shown to other participants when Meet prompts \
                              for a guest name. Trimmed; max 64 characters; no control chars.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the request was validated and accepted.",
                required: true,
            },
            FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Stable UUID for this join attempt; used by the shell as the \
                              webview-window label and per-call data directory.",
                required: true,
            },
            FieldSchema {
                name: "meet_url",
                ty: TypeSchema::String,
                comment: "Normalized Meet URL the shell should navigate to.",
                required: true,
            },
            FieldSchema {
                name: "display_name",
                ty: TypeSchema::String,
                comment: "Trimmed display name to use when joining.",
                required: true,
            },
        ],
    }
}

fn schema_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet",
        function: "unknown",
        description: "Unknown meet controller function.",
        inputs: vec![FieldSchema {
            name: "function",
            ty: TypeSchema::String,
            comment: "Unknown function requested.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Lookup error details.",
            required: true,
        }],
    }
}

fn handle_join_call_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_join_call(params).await })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_call_schema_requires_meet_url_and_display_name() {
        let s = schema_join_call();
        assert_eq!(s.namespace, "meet");
        assert_eq!(s.function, "join_call");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["meet_url", "display_name"]);
    }

    #[test]
    fn registered_controllers_match_schemas() {
        let schema_fns: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        let handler_fns: Vec<_> = all_registered_controllers()
            .into_iter()
            .map(|c| c.schema.function)
            .collect();
        assert_eq!(schema_fns, handler_fns);
        assert_eq!(schema_fns, vec!["join_call"]);
    }

    #[test]
    fn lookup_returns_unknown_for_missing_function() {
        assert_eq!(schemas("nope").function, "unknown");
    }

    #[test]
    fn join_call_outputs_include_request_id() {
        let s = schema_join_call();
        assert!(s
            .outputs
            .iter()
            .any(|f| f.name == "request_id" && f.required));
    }
}
