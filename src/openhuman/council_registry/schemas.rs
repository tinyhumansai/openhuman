//! JSON-RPC controller surface for persistent council definitions.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::council_registry::types::CouncilDefinition;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct CouncilIdParams {
    id: String,
}

#[derive(Debug, Deserialize)]
struct UpsertCouncilParams {
    council: CouncilDefinition,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("get"),
        schemas("upsert"),
        schemas("delete"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("upsert"),
            handler: handle_upsert,
        },
        RegisteredController {
            schema: schemas("delete"),
            handler: handle_delete,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "council_registry",
            function: "list",
            description: "List saved model council definitions for the current workspace.",
            inputs: vec![],
            outputs: vec![json_output("result", "Saved council definitions.")],
        },
        "get" => ControllerSchema {
            namespace: "council_registry",
            function: "get",
            description: "Load one saved model council definition by id.",
            inputs: vec![required_string("id", "Council definition id.")],
            outputs: vec![json_output(
                "result",
                "Council definition, or null if not found.",
            )],
        },
        "upsert" => ControllerSchema {
            namespace: "council_registry",
            function: "upsert",
            description: "Create or update a saved model council definition.",
            inputs: vec![required_json("council", "Council definition payload.")],
            outputs: vec![json_output("result", "Saved council definition.")],
        },
        "delete" => ControllerSchema {
            namespace: "council_registry",
            function: "delete",
            description: "Delete a saved model council definition by id.",
            inputs: vec![required_string("id", "Council definition id.")],
            outputs: vec![json_output("result", "True when a council was deleted.")],
        },
        _ => ControllerSchema {
            namespace: "council_registry",
            function: "unknown",
            description: "Unknown council_registry controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Error message.",
                required: true,
            }],
        },
    }
}

fn handle_list(_: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::council_registry::list_councils(&config)?)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<CouncilIdParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::council_registry::get_council(
            &config, &p.id,
        )?)
    })
}

fn handle_upsert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<UpsertCouncilParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::council_registry::upsert_council(
            &config, p.council,
        )?)
    })
}

fn handle_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<CouncilIdParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::council_registry::delete_council(
            &config, &p.id,
        )?)
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn required_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
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

    #[test]
    fn controller_schema_inventory_is_stable() {
        let schemas = all_controller_schemas();
        let functions: Vec<_> = schemas.iter().map(|schema| schema.function).collect();
        assert_eq!(functions, vec!["list", "get", "upsert", "delete"]);
        assert_eq!(schemas.len(), all_registered_controllers().len());
    }

    #[test]
    fn schemas_expose_expected_rpc_names() {
        let list = schemas("list");
        assert_eq!(
            crate::core::all::rpc_method_name(&list),
            "openhuman.council_registry_list"
        );
        let upsert = schemas("upsert");
        assert_eq!(
            crate::core::all::rpc_method_name(&upsert),
            "openhuman.council_registry_upsert"
        );
        assert!(upsert.inputs.iter().any(|input| input.name == "council"));
    }
}
