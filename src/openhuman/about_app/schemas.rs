use std::str::FromStr;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::about_app::CapabilityCategory;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize, Default)]
struct AboutAppListParams {
    #[serde(default)]
    category: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AboutAppLookupParams {
    id: String,
}

#[derive(Debug, Deserialize)]
struct AboutAppSearchParams {
    query: String,
}

pub fn all_about_app_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        about_app_schemas("about_app_list"),
        about_app_schemas("about_app_lookup"),
        about_app_schemas("about_app_search"),
    ]
}

pub fn all_about_app_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: about_app_schemas("about_app_list"),
            handler: handle_about_app_list,
        },
        RegisteredController {
            schema: about_app_schemas("about_app_lookup"),
            handler: handle_about_app_lookup,
        },
        RegisteredController {
            schema: about_app_schemas("about_app_search"),
            handler: handle_about_app_search,
        },
    ]
}

pub fn about_app_schemas(function: &str) -> ControllerSchema {
    match function {
        "about_app_list" => ControllerSchema {
            namespace: "about_app",
            function: "list",
            description: "List all user-facing app capabilities, optionally filtered by category.",
            inputs: vec![optional_category(
                "category",
                "Optional capability category filter.",
            )],
            outputs: vec![capabilities_output(
                "capabilities",
                "Capability catalog entries matching the list filter.",
            )],
        },
        "about_app_lookup" => ControllerSchema {
            namespace: "about_app",
            function: "lookup",
            description: "Look up one user-facing capability by its stable id.",
            inputs: vec![required_string(
                "id",
                "Capability id, such as local_ai.download_model.",
            )],
            outputs: vec![capability_output(
                "capability",
                "One capability entry for the requested id.",
            )],
        },
        "about_app_search" => ControllerSchema {
            namespace: "about_app",
            function: "search",
            description: "Search user-facing capabilities by keyword.",
            inputs: vec![required_string("query", "Keyword query to search for.")],
            outputs: vec![capabilities_output(
                "capabilities",
                "Capability catalog entries matching the search query.",
            )],
        },
        _ => ControllerSchema {
            namespace: "about_app",
            function: "unknown",
            description: "Unknown about_app controller.",
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

fn handle_about_app_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<AboutAppListParams>(params)?;
        let category = payload
            .category
            .as_deref()
            .map(CapabilityCategory::from_str)
            .transpose()?;

        tracing::debug!(?category, "[about_app] list capabilities");
        to_json(crate::openhuman::about_app::list_capabilities(category))
    })
}

fn handle_about_app_lookup(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<AboutAppLookupParams>(params)?;
        tracing::debug!(id = %payload.id, "[about_app] lookup capability");
        to_json(crate::openhuman::about_app::lookup_capability(&payload.id)?)
    })
}

fn handle_about_app_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<AboutAppSearchParams>(params)?;
        tracing::debug!(query = %payload.query, "[about_app] search capabilities");
        to_json(crate::openhuman::about_app::search_capabilities(
            &payload.query,
        ))
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_category(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
            variants: CapabilityCategory::ALL
                .iter()
                .map(|category| category.as_str())
                .collect(),
        })),
        comment,
        required: false,
    }
}

fn capability_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Ref("Capability"),
        comment,
        required: true,
    }
}

fn capabilities_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Capability"))),
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_names_are_stable() {
        let list = about_app_schemas("about_app_list");
        assert_eq!(list.namespace, "about_app");
        assert_eq!(list.function, "list");

        let lookup = about_app_schemas("about_app_lookup");
        assert_eq!(lookup.namespace, "about_app");
        assert_eq!(lookup.function, "lookup");

        let search = about_app_schemas("about_app_search");
        assert_eq!(search.namespace, "about_app");
        assert_eq!(search.function, "search");
    }

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_about_app_controller_schemas().len(),
            all_about_app_registered_controllers().len()
        );
    }
}
