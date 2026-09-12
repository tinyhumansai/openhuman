//! Controller schemas and handler registrations for the embeddings domain.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::ops as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_settings"),
        schemas("update_settings"),
        schemas("set_api_key"),
        schemas("clear_api_key"),
        schemas("embed"),
        schemas("test_connection"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_settings"),
            handler: handle_get_settings,
        },
        RegisteredController {
            schema: schemas("update_settings"),
            handler: handle_update_settings,
        },
        RegisteredController {
            schema: schemas("set_api_key"),
            handler: handle_set_api_key,
        },
        RegisteredController {
            schema: schemas("clear_api_key"),
            handler: handle_clear_api_key,
        },
        RegisteredController {
            schema: schemas("embed"),
            handler: handle_embed,
        },
        RegisteredController {
            schema: schemas("test_connection"),
            handler: handle_test_connection,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get_settings" => ControllerSchema {
            namespace: "embeddings",
            function: "get_settings",
            description: "Get current embedding settings and provider catalog.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Embedding settings and provider catalog.")],
        },
        "update_settings" => ControllerSchema {
            namespace: "embeddings",
            function: "update_settings",
            description: "Update embedding provider, model, or dimensions. Requires confirm_wipe when signature changes.",
            inputs: vec![
                optional_string("provider", "Embedding provider slug."),
                optional_string("model", "Model identifier."),
                optional_u64("dimensions", "Output vector dimensions."),
                optional_string("custom_endpoint", "Custom endpoint URL (for custom provider)."),
                optional_u64("rate_limit_per_min", "Rate limit in requests per minute."),
                optional_bool("confirm_wipe", "Confirm memory wipe on signature change."),
            ],
            outputs: vec![json_output("result", "Updated settings.")],
        },
        "set_api_key" => ControllerSchema {
            namespace: "embeddings",
            function: "set_api_key",
            description: "Store an API key for an embedding provider.",
            inputs: vec![
                required_string("provider", "Provider slug (e.g. voyage, openai, cohere, custom)."),
                required_string("api_key", "The API key to store."),
            ],
            outputs: vec![json_output("result", "Storage confirmation.")],
        },
        "clear_api_key" => ControllerSchema {
            namespace: "embeddings",
            function: "clear_api_key",
            description: "Remove the stored API key for an embedding provider.",
            inputs: vec![
                required_string("provider", "Provider slug."),
            ],
            outputs: vec![json_output("result", "Removal confirmation.")],
        },
        "embed" => ControllerSchema {
            namespace: "embeddings",
            function: "embed",
            description: "Generate embeddings for text inputs using the configured provider.",
            inputs: vec![FieldSchema {
                name: "inputs",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Texts to embed.",
                required: true,
            }],
            outputs: vec![json_output("result", "Embedding vectors and metadata.")],
        },
        "test_connection" => ControllerSchema {
            namespace: "embeddings",
            function: "test_connection",
            description: "Test connectivity to the configured or specified embedding provider.",
            inputs: vec![
                optional_string("provider", "Provider slug to test (defaults to current)."),
                optional_string("model", "Model to test (defaults to current)."),
                optional_u64("dimensions", "Dimensions to test (defaults to current)."),
            ],
            outputs: vec![json_output("result", "Connection test result.")],
        },
        _ => ControllerSchema {
            namespace: "embeddings",
            function: "unknown",
            description: "Unknown embeddings controller function.",
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

// ── Param structs ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct UpdateSettingsParams {
    provider: Option<String>,
    model: Option<String>,
    dimensions: Option<usize>,
    custom_endpoint: Option<String>,
    rate_limit_per_min: Option<u32>,
    #[serde(default)]
    confirm_wipe: bool,
}

#[derive(Debug, Deserialize)]
struct SetApiKeyParams {
    provider: String,
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct ClearApiKeyParams {
    provider: String,
}

#[derive(Debug, Deserialize)]
struct EmbedParams {
    inputs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TestConnectionParams {
    provider: Option<String>,
    model: Option<String>,
    dimensions: Option<usize>,
}

// ── Handlers ───────────────────────────────────────────────

fn handle_get_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::rpc::get_settings(&config).await?)
    })
}

fn handle_update_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<UpdateSettingsParams>(params)?;
        to_json(
            super::rpc::update_settings(
                p.provider,
                p.model,
                p.dimensions,
                p.custom_endpoint,
                p.rate_limit_per_min,
                p.confirm_wipe,
            )
            .await?,
        )
    })
}

fn handle_set_api_key(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<SetApiKeyParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::rpc::set_api_key(&config, &p.provider, &p.api_key).await?)
    })
}

fn handle_clear_api_key(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ClearApiKeyParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::rpc::clear_api_key(&config, &p.provider).await?)
    })
}

fn handle_embed(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<EmbedParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::rpc::embed(&config, &p.inputs).await?)
    })
}

fn handle_test_connection(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<TestConnectionParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            super::rpc::test_connection(
                &config,
                p.provider.as_deref(),
                p.model.as_deref(),
                p.dimensions,
            )
            .await?,
        )
    })
}

// ── Helpers ────────────────────────────────────────────────

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
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

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
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

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
