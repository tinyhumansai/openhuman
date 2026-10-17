//! RPC controllers for the local desktop connection.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("status"),
            handler: status,
        },
        RegisteredController {
            schema: schema("set_enabled"),
            handler: set_enabled,
        },
        RegisteredController {
            schema: schema("probe"),
            handler: probe,
        },
        RegisteredController {
            schema: schema("pending"),
            handler: pending,
        },
        RegisteredController {
            schema: schema("confirm"),
            handler: confirm,
        },
    ]
}

fn schema(function: &'static str) -> ControllerSchema {
    ControllerSchema {
        namespace: "desktop",
        function,
        description: match function {
            "status" => "Local desktop connection, module, Jev, and OS permission status.",
            "set_enabled" => "Enable or disable local desktop control on this computer.",
            "pending" => "List desktop actions awaiting a user decision on this computer.",
            "confirm" => "Approve or deny one pending desktop action from a trusted client.",
            _ => "Read-only desktop connection test by listing running apps.",
        },
        inputs: if function == "set_enabled" {
            vec![FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "Whether this computer allows desktop control.",
                required: true,
            }]
        } else if function == "confirm" {
            vec![
                FieldSchema {
                    name: "confirmation_id",
                    ty: TypeSchema::String,
                    comment: "One-use confirmation handle returned by a desktop goal.",
                    required: true,
                },
                FieldSchema {
                    name: "approve",
                    ty: TypeSchema::Bool,
                    comment: "True to approve, false to cancel the pending action.",
                    required: true,
                },
            ]
        } else {
            vec![]
        },
        outputs: vec![FieldSchema {
            name: "result",
            ty: TypeSchema::Json,
            comment: "Desktop connection status or probe result.",
            required: true,
        }],
    }
}

fn status(ctx: DispatchCtx, _: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("config unavailable: {error}"))?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        serde_json::to_value(super::ops::status(&config).await).map_err(|error| error.to_string())
    })
}

fn set_enabled(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let value = params
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "enabled must be a boolean".to_owned())?;
        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("config unavailable: {error}"))?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        let result = super::ops::set_enabled(&config, value).await?;
        serde_json::to_value(result).map_err(|error| error.to_string())
    })
}

fn probe(ctx: DispatchCtx, _: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("config unavailable: {error}"))?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        serde_json::to_value(super::ops::probe(&config).await).map_err(|error| error.to_string())
    })
}

fn pending(ctx: DispatchCtx, _: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("config unavailable: {error}"))?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        serde_json::to_value(super::confirmation::pending()).map_err(|error| error.to_string())
    })
}

fn confirm(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = params
            .get("confirmation_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "confirmation_id is required".to_owned())?;
        let approve = params
            .get("approve")
            .and_then(Value::as_bool)
            .ok_or_else(|| "approve must be a boolean".to_owned())?;
        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("config unavailable: {error}"))?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        super::confirmation::confirm(&config, id, approve).await
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
