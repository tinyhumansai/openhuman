//! RPC controller schemas for the providers domain.
//!
//! Exposes `openhuman.providers_list_models` — fetches the `/models` endpoint
//! of a configured cloud provider and returns the list.

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn to_json<T: Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
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

#[derive(Debug, Serialize)]
struct ModelInfo {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owned_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<u64>,
}

// ── Handler ───────────────────────────────────────────────────────────────────

fn handle_list_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req: ListModelsRequest = deserialize_params(params)?;
        let provider_id = req.provider_id.trim().to_string();

        if provider_id.is_empty() {
            return Err("provider_id must not be empty".to_string());
        }

        log::debug!("[providers][list_models] provider_id={}", provider_id);

        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| e.to_string())?;

        let entry = config
            .cloud_providers
            .iter()
            .find(|e| e.id == provider_id)
            .cloned()
            .ok_or_else(|| format!("no cloud provider with id '{}' found", provider_id))?;

        // Build the /models URL from the provider's endpoint.
        let base = entry.endpoint.trim_end_matches('/');
        let models_url = format!("{}/models", base);

        log::debug!(
            "[providers][list_models] fetching url={} slug={}",
            models_url,
            entry.slug
        );

        // Fetch the API key for this provider.
        let api_key =
            crate::openhuman::providers::factory::lookup_key_for_slug(&entry.slug, &config)
                .unwrap_or_default();

        // Build the HTTP client (reuse the runtime proxy config). Explicit
        // timeouts mirror the other external integrations (composio,
        // multimodal) so a slow/unresponsive provider can't hang the panel.
        let client = crate::openhuman::config::build_runtime_proxy_client_with_timeouts(
            "providers.list_models",
            30,
            10,
        );

        let mut request = client.get(&models_url);

        // Attach auth header per auth_style.
        use crate::openhuman::config::schema::cloud_providers::AuthStyle;
        request = match entry.auth_style {
            AuthStyle::Bearer => {
                if !api_key.is_empty() {
                    request.header("Authorization", format!("Bearer {}", api_key))
                } else {
                    request
                }
            }
            AuthStyle::Anthropic => {
                let mut r = request.header("anthropic-version", "2023-06-01");
                if !api_key.is_empty() {
                    r = r.header("x-api-key", &api_key);
                }
                r
            }
            AuthStyle::OpenhumanJwt | AuthStyle::None => request,
        };

        let response = request
            .send()
            .await
            .map_err(|e| format!("[providers][list_models] HTTP request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let truncated = crate::openhuman::util::truncate_with_ellipsis(&body, 300);
            return Err(format!(
                "provider returned {}: {}",
                status.as_u16(),
                truncated
            ));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("[providers][list_models] failed to parse JSON: {}", e))?;

        // Parse OpenAI-compatible `{ data: [{ id, owned_by? }] }` or
        // Anthropic `{ data: [{ id, display_name }] }`.
        let data = body
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let models: Vec<ModelInfo> = data
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?.as_str()?.to_string();
                let owned_by = item
                    .get("owned_by")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let context_window = item
                    .get("context_length")
                    .or_else(|| item.get("context_window"))
                    .and_then(|v| v.as_u64());
                Some(ModelInfo {
                    id,
                    owned_by,
                    context_window,
                })
            })
            .collect();

        log::info!(
            "[providers][list_models] slug={} fetched {} models",
            entry.slug,
            models.len()
        );

        to_json(RpcOutcome::new(
            serde_json::json!({ "models": models }),
            vec![format!("fetched {} models", models.len())],
        ))
    })
}
