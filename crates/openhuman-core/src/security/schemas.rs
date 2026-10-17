use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("policy_info")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("policy_info"),
        handler: handle_policy_info,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "policy_info" => ControllerSchema {
            namespace: "security",
            function: "policy_info",
            description: "Return the active security/autonomy policy used by the core runtime.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "policy",
                ty: TypeSchema::Json,
                comment: "Security policy metadata and feature flags.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "security",
            function: "unknown",
            description: "Unknown security controller function.",
            inputs: vec![],
            outputs: vec![],
        },
    }
}

fn handle_policy_info(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[security][rpc] policy_info enter");
        match crate::security::rpc::load_and_get_security_policy_info(&ctx).await {
            Ok(outcome) => {
                log::debug!("[security][rpc] policy_info ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[security][rpc] policy_info failed: {err}");
                Err(err)
            }
        }
    })
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
