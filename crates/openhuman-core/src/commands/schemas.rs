//! RPC/CLI controller surface for the `commands` domain: one read-only
//! method, `commands.list`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("list")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("list"),
        handler: handle_list,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "commands",
            function: "list",
            description: "List every command the chat composer's slash-command menu can \
                          offer: the fixed built-ins (/new, /clear, /plan, /build, /goal, \
                          /todo, /stop) merged with the live skills.list and flows.list \
                          catalogs. Read-only — naming a command here does not run it; \
                          execution stays on the RPCs the frontend already uses \
                          (skills.run / flows.run / the built-in's own RPC).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "commands",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of {id, label, description, kind: \"builtin\"|\"skill\"|\
                          \"workflow\", insert?}.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "commands",
            function: "unknown",
            description: "Unknown commands controller function.",
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

fn handle_list(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::ops::commands_list(&ctx)
            .await?
            .into_cli_compatible_json()
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
