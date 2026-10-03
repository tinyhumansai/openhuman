//! Handlers for model and provider settings: resolution, status, client
//! config, model and local-runtime settings updates, model listing, device
//! auth errors, and diagnostics.

use serde::de::Deserializer;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{deserialize_params, to_json};
use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;
use crate::core::Outcome;

#[derive(Debug, Deserialize)]
pub(super) struct InferenceResolveModelParams {
    hint: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceModelRouteUpdate {
    hint: String,
    model: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceCloudProviderUpdate {
    id: Option<String>,
    slug: String,
    #[serde(default)]
    label: Option<String>,
    endpoint: String,
    #[serde(default)]
    auth_style: Option<String>,
    #[serde(rename = "type", default)]
    legacy_type: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceUpdateModelSettingsParams {
    api_url: Option<String>,
    inference_url: Option<String>,
    api_key: Option<String>,
    default_model: Option<String>,
    default_temperature: Option<f64>,
    model_routes: Option<Vec<InferenceModelRouteUpdate>>,
    cloud_providers: Option<Vec<InferenceCloudProviderUpdate>>,
    #[serde(default)]
    model_registry: Option<Vec<crate::config::schema::ModelRegistryEntry>>,
    primary_cloud: Option<String>,
    chat_provider: Option<String>,
    reasoning_provider: Option<String>,
    agentic_provider: Option<String>,
    coding_provider: Option<String>,
    vision_provider: Option<String>,
    memory_provider: Option<String>,
    embeddings_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceUpdateLocalSettingsParams {
    runtime_enabled: Option<bool>,
    opt_in_confirmed: Option<bool>,
    provider: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_json")]
    base_url: Option<Value>,
    model_id: Option<String>,
    chat_model_id: Option<String>,
    usage_embeddings: Option<bool>,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceListModelsParams {
    provider_id: String,
}

pub(super) fn handle_inference_resolve_model(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceResolveModelParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let resolved =
            crate::inference::provider::factory::resolve_model_for_hint(&p.hint, &config);
        // Whether the resolved model accepts image input — drives the chat UI's
        // image-attachment affordance. Managed OpenHuman tiers consult the
        // core-owned per-tier map, which returns `true` for the reasoning and
        // vision tiers (and their `hint:` aliases) and `false` for every other
        // tier; custom/BYOK models are covered by the user's per-model
        // `model_registry.vision` flag.
        let vision = crate::inference::model_context::model_supports_vision(&resolved, &config);
        to_json(Outcome::new(
            serde_json::json!({ "model": resolved, "vision": vision }),
            vec![],
        ))
    })
}

pub(super) fn handle_inference_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::inference::rpc::inference_status(&config).await?)
    })
}

pub(super) fn handle_inference_get_client_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(crate::inference::rpc::inference_get_client_config().await?) })
}

pub(super) fn handle_inference_update_model_settings(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<InferenceUpdateModelSettingsParams>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_url: update.api_url,
            inference_url: update.inference_url,
            api_key: update.api_key,
            default_model: update.default_model,
            default_temperature: update.default_temperature,
            model_routes: update.model_routes.map(|routes| {
                routes
                    .into_iter()
                    .map(|route| crate::config::ModelRouteConfig {
                        hint: route.hint,
                        model: route.model,
                    })
                    .collect()
            }),
            cloud_providers: update
                .cloud_providers
                .map(|entries| {
                    use crate::config::schema::cloud_providers::{
                        generate_provider_id, is_slug_reserved, migrate_legacy_fields, AuthStyle,
                        CloudProviderCreds,
                    };
                    let reserved_count = entries
                        .iter()
                        .filter(|e| {
                            let t = e.slug.trim();
                            !t.is_empty() && is_slug_reserved(t)
                        })
                        .count();
                    if reserved_count > 0 {
                        log::debug!(
                            "[inference] update_model_settings: dropping {} reserved cloud provider slug(s)",
                            reserved_count
                        );
                    }
                    entries
                        .into_iter()
                        // Silently drop entries whose (non-empty) slug is reserved —
                        // typically the migration-seeded "openhuman" / "cloud" /
                        // "pid" built-ins that the frontend echoes back on every
                        // save (see `migrations::unify_ai_provider_settings`).
                        // Empty slugs still fall through so the explicit
                        // validation error below fires for actual frontend
                        // bugs. `apply_model_settings` re-injects the existing
                        // reserved entries from the stored config so they
                        // aren't dropped on save.
                        .filter(|entry| {
                            let trimmed = entry.slug.trim();
                            trimmed.is_empty() || !is_slug_reserved(trimmed)
                        })
                        .map(|entry| {
                            let slug = entry.slug.trim().to_string();
                            if slug.is_empty() {
                                return Err("cloud provider slug must not be empty".to_string());
                            }
                            let auth_style = match entry
                                .auth_style
                                .as_deref()
                                .unwrap_or("bearer")
                                .to_ascii_lowercase()
                                .as_str()
                            {
                                "bearer" => AuthStyle::Bearer,
                                "anthropic" => AuthStyle::Anthropic,
                                "openhuman_jwt" | "openhumanjwt" => AuthStyle::OpenhumanJwt,
                                "none" => AuthStyle::None,
                                other => {
                                    return Err(format!(
                                        "unknown auth_style '{}'; valid: bearer, anthropic, openhuman_jwt, none",
                                        other
                                    ))
                                }
                            };
                            let id = entry
                                .id
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| generate_provider_id(&slug));
                            let label = entry
                                .label
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| slug.clone());
                            let mut provider = CloudProviderCreds {
                                id,
                                slug,
                                label,
                                endpoint: entry.endpoint,
                                auth_style,
                                legacy_type: entry.legacy_type,
                                default_model: entry.default_model,
                            };
                            migrate_legacy_fields(&mut provider);
                            Ok(provider)
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?,
            model_registry: update.model_registry,
            primary_cloud: update.primary_cloud,
            chat_provider: update.chat_provider,
            reasoning_provider: update.reasoning_provider,
            agentic_provider: update.agentic_provider,
            coding_provider: update.coding_provider,
            vision_provider: update.vision_provider,
            memory_provider: update.memory_provider,
            embeddings_provider: update.embeddings_provider,
        };
        to_json(crate::inference::rpc::inference_update_model_settings(patch).await?)
    })
}

pub(super) fn handle_inference_update_local_settings(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<InferenceUpdateLocalSettingsParams>(params)?;
        let base_url = match update.base_url {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(value)) => Some(Some(value)),
            Some(_) => return Err("invalid params: base_url must be a string or null".to_string()),
        };
        let patch = config_rpc::LocalAiSettingsPatch {
            runtime_enabled: update.runtime_enabled,
            opt_in_confirmed: update.opt_in_confirmed,
            provider: update.provider,
            base_url,
            model_id: update.model_id,
            chat_model_id: update.chat_model_id,
            usage_embeddings: update.usage_embeddings,
            api_key: update.api_key,
        };
        to_json(crate::inference::rpc::inference_update_local_settings(patch).await?)
    })
}

pub(super) fn handle_inference_list_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request = deserialize_params::<InferenceListModelsParams>(params)?;
        to_json(crate::inference::rpc::inference_list_models(&request.provider_id).await?)
    })
}

pub(super) fn handle_inference_provider_auth_errors(
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move { to_json(crate::inference::rpc::inference_provider_auth_errors().await?) })
}

pub(super) fn handle_inference_diagnostics(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::inference::rpc::inference_diagnostics(&config).await?)
    })
}

fn deserialize_present_json<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}
