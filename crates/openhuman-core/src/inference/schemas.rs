//! The `inference` RPC controller registry: schemas, handlers, and the
//! wiring between them.

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;

mod catalog;
mod claude_code_handlers;
mod prompt_handlers;
mod settings_handlers;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;
use crate::core::Outcome;

pub use catalog::schemas;
use claude_code_handlers::*;
use prompt_handlers::*;
use settings_handlers::*;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("resolve_model"),
        schemas("status"),
        schemas("get_client_config"),
        schemas("update_model_settings"),
        schemas("update_local_settings"),
        schemas("list_models"),
        schemas("provider_auth_errors"),
        schemas("diagnostics"),
        schemas("summarize"),
        schemas("prompt"),
        schemas("vision_prompt"),
        schemas("test_provider_model"),
        schemas("analyze_sentiment"),
        schemas("claude_code_status"),
        schemas("claude_code_auth_status"),
        schemas("claude_code_settings"),
        schemas("claude_code_set_full_access"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("resolve_model"),
            handler: handle_inference_resolve_model,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_inference_status,
        },
        RegisteredController {
            schema: schemas("get_client_config"),
            handler: handle_inference_get_client_config,
        },
        RegisteredController {
            schema: schemas("update_model_settings"),
            handler: handle_inference_update_model_settings,
        },
        RegisteredController {
            schema: schemas("update_local_settings"),
            handler: handle_inference_update_local_settings,
        },
        RegisteredController {
            schema: schemas("list_models"),
            handler: handle_inference_list_models,
        },
        RegisteredController {
            schema: schemas("provider_auth_errors"),
            handler: handle_inference_provider_auth_errors,
        },
        RegisteredController {
            schema: schemas("diagnostics"),
            handler: handle_inference_diagnostics,
        },
        RegisteredController {
            schema: schemas("summarize"),
            handler: handle_inference_summarize,
        },
        RegisteredController {
            schema: schemas("prompt"),
            handler: handle_inference_prompt,
        },
        RegisteredController {
            schema: schemas("vision_prompt"),
            handler: handle_inference_vision_prompt,
        },
        RegisteredController {
            schema: schemas("test_provider_model"),
            handler: handle_inference_test_provider_model,
        },
        RegisteredController {
            schema: schemas("analyze_sentiment"),
            handler: handle_inference_analyze_sentiment,
        },
        RegisteredController {
            schema: schemas("claude_code_status"),
            handler: handle_inference_claude_code_status,
        },
        RegisteredController {
            schema: schemas("claude_code_auth_status"),
            handler: handle_inference_claude_code_auth_status,
        },
        RegisteredController {
            schema: schemas("claude_code_settings"),
            handler: handle_inference_claude_code_settings,
        },
        RegisteredController {
            schema: schemas("claude_code_set_full_access"),
            handler: handle_inference_claude_code_set_full_access,
        },
    ]
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
