//! RPC controller schemas for the providers domain.
//!
//! Exposes `openhuman.providers_list_models` — fetches the `/models` endpoint
//! of a configured cloud provider and returns the list.

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use serde::Deserialize;
use serde_json::{Map, Value};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn to_json<T: serde::Serialize>(outcome: crate::rpc::RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn deserialize_params<T: for<'de> Deserialize<'de>>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())
}

// ── Schema catalog ────────────────────────────────────────────────────────────

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![list_models_schema()]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: list_models_schema(),
        handler: handle_list_models,
    }]
}

fn list_models_schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "providers",
        function: "list_models",
        description: "Fetch the available model list from a configured cloud provider's /models API.",
        inputs: vec![
            FieldSchema {
                name: "provider_id",
                ty: TypeSchema::String,
                comment: "Opaque id of the cloud_providers entry to query.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "models",
                ty: TypeSchema::Json,
                comment: "Array of { id, owned_by?, context_window? } model descriptors returned by the provider.",
                required: true,
            },
        ],
    }
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ListModelsRequest {
    provider_id: String,
}

// ── Handler ───────────────────────────────────────────────────────────────────

fn handle_list_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req: ListModelsRequest = deserialize_params(params)?;
        to_json(
            crate::openhuman::inference::provider::ops::list_configured_models(&req.provider_id)
                .await?,
        )
    })
}
