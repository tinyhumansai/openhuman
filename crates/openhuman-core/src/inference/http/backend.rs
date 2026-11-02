//! Host [`ChatBackend`] for the OpenAI-compatible `/v1` endpoint.
//!
//! The generic handlers (wire types, SSE framing, error envelope) live in
//! `tinyinference_llm::providers::openai::server_http`. This adapter only
//! resolves a request's `model` against OpenHuman's `Config` and the provider
//! factory, and lists the configured models.
//!
//! ## Provider routing
//!
//! - `"ollama:<model>"` or a bare model name -> local Ollama
//! - `"<slug>:<model>"` -> cloud provider entry by slug
//! - `"openhuman"` -> OpenHuman backend (session JWT)

use async_trait::async_trait;
use tinyinference_llm::providers::openai::server_http::{
    BackendFailure, ChatBackend, ListedModel, ResolvedChat,
};
use tinyinference_providers::route::{bare_model_route, route_owner};
use tracing::{debug, warn};

use crate::config::Config;

const LOG_PREFIX: &str = "[inference::http]";

/// Resolves `/v1` requests against the loaded OpenHuman [`Config`].
pub(super) struct HostChatBackend;

/// Bare model names (no `:`, not the `openhuman` sentinel) are local Ollama models.
pub(super) fn provider_string_for(model: &str) -> String {
    bare_model_route(model)
}

#[async_trait]
impl ChatBackend for HostChatBackend {
    async fn resolve(
        &self,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ResolvedChat, BackendFailure> {
        let config = Config::load_or_init().await.map_err(|e| {
            warn!("{LOG_PREFIX} resolve: config load failed: {e}");
            BackendFailure::internal(format!("config load failed: {e}"))
        })?;
        let provider_string = provider_string_for(model);
        let raw = temperature.unwrap_or(config.default_temperature);
        let (chat_model, model_id) =
            crate::inference::provider::create_chat_model_from_string_with_model_id(
                "agentic",
                &provider_string,
                &config,
                raw,
            )
            .await
            .map_err(|e| {
                warn!("{LOG_PREFIX} resolve: provider build failed: {e}");
                BackendFailure::invalid_request(format!("provider error: {e}"))
            })?;

        // Reasoning models reject `temperature`; the provider layer omits it on
        // the wire, so only log the drop here.
        let suppressed = tinyinference_llm::model::effective_temperature(
            &model_id,
            Some(raw),
            None,
            &config.temperature_unsupported_models,
        );
        if suppressed.is_none() && temperature.is_some() {
            warn!(
                model = %model_id,
                requested_temperature = raw,
                "{LOG_PREFIX} dropping caller-supplied temperature — model is on temperature_unsupported_models list"
            );
        }
        Ok(ResolvedChat {
            chat_model,
            model_id,
            temperature: raw,
        })
    }

    async fn list_models(&self) -> Result<Vec<ListedModel>, BackendFailure> {
        let config = Config::load_or_init().await.map_err(|e| {
            warn!("{LOG_PREFIX} models: config load failed: {e}");
            BackendFailure::internal(format!("config load failed: {e}"))
        })?;
        let models = configured_models(&config);
        debug!(model_count = models.len(), "{LOG_PREFIX} models: ok");
        Ok(models)
    }
}

/// Every model the config advertises (duplicates are removed upstream).
pub(super) fn configured_models(config: &Config) -> Vec<ListedModel> {
    let mut out = Vec::new();
    let mut push = |id: String, owned_by: &str| out.push(ListedModel::new(id, owned_by));

    // Stable managed-router sentinel: OpenHuman keeps selecting the effective
    // upstream model from the current routing config.
    push("openhuman".to_string(), "openhuman");

    if let Some(default_model) = config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        // A configured default is known to be managed even when its model id
        // contains a colon (for example `qwen3:14b`).
        push(route_owner(default_model).0, "openhuman");
    }

    for cp in &config.cloud_providers {
        if let Some(ref model) = cp.default_model {
            push(route_owner(&format!("{}:{model}", cp.slug)).0, &cp.slug);
        }
    }

    if !config.local_ai.chat_model_id.is_empty() {
        push(
            route_owner(&format!("ollama:{}", config.local_ai.chat_model_id)).0,
            "ollama",
        );
    }

    for provider_string in [
        config.chat_provider.as_deref(),
        config.reasoning_provider.as_deref(),
        config.agentic_provider.as_deref(),
        config.coding_provider.as_deref(),
        config.vision_provider.as_deref(),
        config.memory_provider.as_deref(),
        config.embeddings_provider.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty() && *value != "cloud" && *value != "openhuman")
    {
        if let Some((slug, _)) = provider_string.split_once(':') {
            if slug != "openhuman" {
                let (id, _) = route_owner(provider_string);
                push(id, slug);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
