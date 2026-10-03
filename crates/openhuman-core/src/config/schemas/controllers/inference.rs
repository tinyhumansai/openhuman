//! Handlers for config snapshot reads and model, memory, runtime, and local-AI inference settings.

use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;

use super::super::helpers::{
    deserialize_params, to_json, LocalAiSettingsUpdate, MemorySettingsUpdate, ModelSettingsUpdate,
    RuntimeSettingsUpdate,
};

pub(super) fn handle_get_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::load_and_get_config_snapshot().await?) })
}

pub(super) fn handle_get_client_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] get_client_config enter");
        match config_rpc::load_and_get_client_config_snapshot().await {
            Ok(snapshot) => to_json(snapshot),
            Err(err) => {
                log::warn!("[config][rpc] get_client_config load failed: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_update_model_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ModelSettingsUpdate>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_url: update.api_url,
            inference_url: update.inference_url,
            api_key: update.api_key,
            default_model: update.default_model,
            default_temperature: update.default_temperature,
            model_routes: update.model_routes.map(|routes| {
                routes
                    .into_iter()
                    .map(|r| crate::config::ModelRouteConfig {
                        hint: r.hint,
                        model: r.model,
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
                            "[config] update_model_settings: dropping {} reserved cloud provider slug(s)",
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
                        .filter(|e| {
                            let trimmed = e.slug.trim();
                            trimmed.is_empty() || !is_slug_reserved(trimmed)
                        })
                        .map(|e| {
                            let slug = e.slug.trim().to_string();
                            if slug.is_empty() {
                                return Err(
                                    "cloud provider slug must not be empty".to_string()
                                );
                            }
                            let auth_style = match e
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
                            let id = e
                                .id
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| generate_provider_id(&slug));
                            let label = e
                                .label
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| slug.clone());
                            let mut entry = CloudProviderCreds {
                                id,
                                slug,
                                label,
                                endpoint: e.endpoint,
                                auth_style,
                                legacy_type: e.legacy_type,
                                default_model: e.default_model,
                            };
                            // Apply any remaining legacy-field migration.
                            migrate_legacy_fields(&mut entry);
                            Ok(entry)
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?,
            // The config-domain RPC doesn't carry a model-registry payload — the
            // per-model vision registry is updated via the inference-domain
            // `inference_update_model_settings` path.
            model_registry: None,
            primary_cloud: update.primary_cloud,
            chat_provider: update.chat_provider,
            reasoning_provider: update.reasoning_provider,
            agentic_provider: update.agentic_provider,
            coding_provider: update.coding_provider,
            vision_provider: update.vision_provider,
            memory_provider: update.memory_provider,
            embeddings_provider: update.embeddings_provider,
        };
        to_json(config_rpc::load_and_apply_model_settings(patch).await?)
    })
}

pub(super) fn handle_update_memory_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<MemorySettingsUpdate>(params)?;
        let patch = config_rpc::MemorySettingsPatch {
            embedding_provider: update.embedding_provider,
            embedding_model: update.embedding_model,
            embedding_dimensions: update.embedding_dimensions,
        };
        to_json(config_rpc::load_and_apply_memory_settings(patch).await?)
    })
}

pub(super) fn handle_update_runtime_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<RuntimeSettingsUpdate>(params)?;
        let patch = config_rpc::RuntimeSettingsPatch {
            kind: update.kind,
            reasoning_enabled: update.reasoning_enabled,
            reasoning_effort: update.reasoning_effort,
        };
        to_json(config_rpc::load_and_apply_runtime_settings(patch).await?)
    })
}

pub(super) fn handle_update_local_ai_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<LocalAiSettingsUpdate>(params)?;
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
        to_json(config_rpc::load_and_apply_local_ai_settings(patch).await?)
    })
}

pub(super) fn handle_resolve_api_url(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::load_and_resolve_api_url().await?) })
}

pub(super) fn handle_get_runtime_flags(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_runtime_flags()) })
}
