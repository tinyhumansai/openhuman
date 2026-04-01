use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::UpdateMode;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct SetPolicyParams {
    mode: UpdateMode,
    check_interval_hours: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DismissParams {
    version: String,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("status"),
        schemas("set_policy"),
        schemas("dismiss"),
        schemas("check"),
        schemas("apply"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("set_policy"),
            handler: handle_set_policy,
        },
        RegisteredController {
            schema: schemas("dismiss"),
            handler: handle_dismiss,
        },
        RegisteredController {
            schema: schemas("check"),
            handler: handle_check,
        },
        RegisteredController {
            schema: schemas("apply"),
            handler: handle_apply,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "update",
            function: "status",
            description: "Read core binary update policy and latest known status.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("UpdateCheckStatus"),
                comment: "Current update status snapshot.",
                required: true,
            }],
        },
        "set_policy" => ControllerSchema {
            namespace: "update",
            function: "set_policy",
            description: "Set core binary update mode and optional check cadence.",
            inputs: vec![
                FieldSchema {
                    name: "mode",
                    ty: TypeSchema::Enum {
                        variants: vec!["auto", "prompt", "manual"],
                    },
                    comment: "Update mode.",
                    required: true,
                },
                FieldSchema {
                    name: "check_interval_hours",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Optional polling interval in hours.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("UpdateCheckStatus"),
                comment: "Updated status snapshot.",
                required: true,
            }],
        },
        "dismiss" => ControllerSchema {
            namespace: "update",
            function: "dismiss",
            description: "Dismiss an available update version so prompt mode stops prompting for it.",
            inputs: vec![FieldSchema {
                name: "version",
                ty: TypeSchema::String,
                comment: "The version string to dismiss.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("UpdateCheckStatus"),
                comment: "Updated status snapshot.",
                required: true,
            }],
        },
        "check" => ControllerSchema {
            namespace: "update",
            function: "check",
            description: "Manually check for a newer core binary release.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("UpdateCheckStatus"),
                comment: "Updated status after release check.",
                required: true,
            }],
        },
        "apply" => ControllerSchema {
            namespace: "update",
            function: "apply",
            description: "Download, verify, and stage the newest core binary update.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Ref("UpdateApplyStatus"),
                comment: "Staged update details.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "update",
            function: "unknown",
            description: "Unknown update controller function.",
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
    Box::pin(async { to_json(crate::openhuman::update::rpc::update_status().await?) })
}

fn handle_set_policy(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: SetPolicyParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        to_json(
            crate::openhuman::update::rpc::update_set_policy(
                payload.mode,
                payload.check_interval_hours,
            )
            .await?,
        )
    })
}

fn handle_dismiss(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: DismissParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        to_json(crate::openhuman::update::rpc::update_dismiss(payload.version).await?)
    })
}

fn handle_check(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::update::rpc::update_check().await?) })
}

fn handle_apply(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::update::rpc::update_apply().await?) })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
