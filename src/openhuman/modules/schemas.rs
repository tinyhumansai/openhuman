//! The `modules` RPC namespace.
//!
//! Read-only plus one deliberate action. `list` and `status` report what this
//! build knows and what it has loaded; `load` forces a lazy module to resolve
//! now, which is what a settings screen offering "install this now" needs.
//!
//! There is no `unload`, and there cannot be: tinybus never unloads a library.
//! There is also no way to name an artifact over RPC — the loadable set is
//! compiled into [`super::registry`], and a method that could point the loader at
//! an arbitrary path would turn this namespace into remote code execution.

use serde_json::{Map, Value};

use super::ops;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("list"), schemas("status"), schemas("load")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("load"),
            handler: handle_load,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "modules",
            function: "list",
            description: "List every loadable module this build knows, with its state.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "modules",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ModuleStatus"))),
                comment: "Status of each known module.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "modules",
            function: "status",
            description: "Report the state of one module by id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Registry identifier, e.g. `tinydocs`.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "module",
                ty: TypeSchema::Ref("ModuleStatus"),
                comment: "Status of the requested module.",
                required: true,
            }],
        },
        "load" => ControllerSchema {
            namespace: "modules",
            function: "load",
            description: "Resolve and load a module now instead of on first use.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Registry identifier, e.g. `tinydocs`.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "module",
                ty: TypeSchema::Ref("ModuleStatus"),
                comment: "Status after the load attempt.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "modules",
            function: "unknown",
            description: "Unknown modules controller function.",
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

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        Ok(serde_json::json!({ "modules": ops::list(&config) }))
    })
}

fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = string_param(&params, "id").ok_or("`id` is required")?;
        let config = config_rpc::load_config_with_timeout().await?;
        match ops::list(&config).into_iter().find(|m| m.id == id) {
            Some(module) => Ok(serde_json::json!({ "module": module })),
            None => Err(format!("unknown module '{id}'")),
        }
    })
}

fn handle_load(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = string_param(&params, "id").ok_or("`id` is required")?;
        let config = config_rpc::load_config_with_timeout().await?;
        // A failed load is reported through the returned status rather than as
        // an RPC error: the caller asked "what happened", and the answer is a
        // state plus a reason, not a transport failure.
        if let Err(reason) = ops::ensure_loaded(&config, &id).await {
            log::warn!("[modules] explicit load of '{id}' failed: {reason}");
        }
        match ops::list(&config).into_iter().find(|m| m.id == id) {
            Some(module) => Ok(serde_json::json!({ "module": module })),
            None => Err(format!("unknown module '{id}'")),
        }
    })
}

/// A required string parameter, rejecting a blank one.
fn string_param(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{all_controller_schemas, all_registered_controllers, schemas, string_param};
    use serde_json::{Map, Value};

    #[test]
    fn every_schema_is_in_the_modules_namespace() {
        for schema in all_controller_schemas() {
            assert_eq!(schema.namespace, "modules");
            assert_ne!(
                schema.function, "unknown",
                "an advertised function fell through to the unknown arm"
            );
        }
    }

    #[test]
    fn registered_controllers_match_the_advertised_schemas() {
        // Two lists that must agree: one drives `/schema`, the other dispatch.
        let advertised: Vec<&str> = all_controller_schemas()
            .iter()
            .map(|s| s.function)
            .collect();
        let registered: Vec<&str> = all_registered_controllers()
            .iter()
            .map(|c| c.schema.function)
            .collect();
        assert_eq!(advertised, registered);
    }

    #[test]
    fn an_unknown_function_falls_through_to_the_unknown_arm() {
        assert_eq!(schemas("nope").function, "unknown");
    }

    #[test]
    fn a_blank_or_missing_id_is_not_a_parameter() {
        let mut params = Map::new();
        assert_eq!(string_param(&params, "id"), None);
        params.insert("id".to_string(), Value::String("   ".to_string()));
        assert_eq!(string_param(&params, "id"), None);
        params.insert("id".to_string(), Value::String(" tinydocs ".to_string()));
        assert_eq!(string_param(&params, "id"), Some("tinydocs".to_string()));
    }
}
