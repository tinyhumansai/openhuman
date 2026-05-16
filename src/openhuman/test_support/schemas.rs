use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::test_support::rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("reset")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("reset"),
        handler: handle_reset,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "reset" => ControllerSchema {
            namespace: "test",
            function: "reset",
            description:
                "Wipe persistent sidecar state in-place: clears auth, onboarding, and cron jobs. \
                 E2E specs call this between tests so each starts from a fresh-install baseline.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "summary",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "cron_jobs_removed",
                            ty: TypeSchema::U64,
                            comment: "Number of cron jobs deleted from the workspace database.",
                            required: true,
                        },
                        FieldSchema {
                            name: "onboarding_was_completed",
                            ty: TypeSchema::Bool,
                            comment: "Whether chat_onboarding_completed was true before the reset.",
                            required: true,
                        },
                        FieldSchema {
                            name: "api_key_was_set",
                            ty: TypeSchema::Bool,
                            comment: "Whether an api_key was present before the reset.",
                            required: true,
                        },
                        FieldSchema {
                            name: "active_user_cleared",
                            ty: TypeSchema::Bool,
                            comment: "Whether active_user.toml was successfully removed.",
                            required: true,
                        },
                    ],
                },
                comment: "Summary of what was wiped.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "test",
            function: "unknown",
            description: "Unknown test-support controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_reset(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::reset().await?) })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
