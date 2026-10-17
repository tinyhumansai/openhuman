//! Resolving a turn's context window from the provider.
//!
//! The window drives pre-dispatch trimming and the compaction trigger, so a
//! wrong one is expensive both ways: too small and a 1M-token model compacts
//! at 128k; too large and the provider rejects the request. The order is:
//!
//! 1. **Config override**: a non-zero `context_window` on the matching
//!    `config.model_registry` entry.
//! 2. **Provider-reported**: the provider's own model listing, discovered
//!    through `tinyinference_llm::model::discover` (bounded, cached per
//!    endpoint and model), lowered by any window the provider stated in a
//!    context-overflow error on that endpoint (learned by the chat adapters).
//!    Ollama's OpenAI-compatible listing carries no window, so for the local
//!    Ollama provider and any endpoint that looks like an Ollama server
//!    (including a custom OpenAI-compatible provider at `:11434/v1`) the same
//!    discovery asks the native `POST /api/show`; its result, or its failure,
//!    is cached and the whole lookup is time-bounded.
//! 3. **Local runtime profile** for local providers (Ollama, LM Studio, ...).
//! 4. **Static guess**: the tier aliases, the cost catalog, and the generic
//!    id-pattern table ([`super::model_context::static_context_window_for_model`]),
//!    logged as a guess at `warn` once per model.
//!
//! The provider-sourced value is remembered per model, so the synchronous
//! [`super::model_context::context_window_for_model`] (usage meters, the
//! context breakdown) reports the same window the turn used.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use tinyinference_llm::model::context_window::{ContextWindowInput, ContextWindowResolver};
use tinyinference_llm::model::context_window::{ResolvedWindow, WindowSource};
use tinyinference_llm::model::discover::{
    model_limits_cache, DiscoveryRequest, ModelLimitsCache, ModelListingFetcher,
};

use crate::config::Config;

/// The user's explicit window for `model`, from `config.model_registry`.
pub(crate) fn config_override(model: &str, config: &Config) -> Option<u64> {
    let model = model.trim();
    config
        .model_registry
        .iter()
        .find(|entry| entry.id.trim() == model && entry.context_window > 0)
        .map(|entry| u64::from(entry.context_window))
}

/// The process-wide resolver: precedence and the per-route/endpoint memory of
/// provider-sourced windows live in `tinyinference-llm`.
fn resolver() -> &'static ContextWindowResolver {
    static RESOLVER: OnceLock<ContextWindowResolver> = OnceLock::new();
    RESOLVER.get_or_init(ContextWindowResolver::new)
}

/// The last provider-sourced window resolved for `model` in this process.
pub(crate) fn remembered_window(provider: &str, model: &str) -> Option<u64> {
    resolver().remembered_window(provider, model)
}

/// Logs a static guess at `warn` the first time per model, `debug` after.
fn warn_static_guess(model: &str, window: Option<u64>, provider: &str) {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let first = WARNED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(model.to_string());
    let route = provider.split(':').next().unwrap_or_default();
    if first {
        tracing::warn!(
            model,
            route,
            context_window = ?window,
            "[model_context] context window is a static guess — the provider did not report one"
        );
    } else {
        tracing::debug!(
            model,
            route,
            context_window = ?window,
            "[model_context] context window is a static guess"
        );
    }
}

/// Discovery request for the built-in local Ollama provider: its OpenAI
/// listing carries no window, so the native `/api/show` probe is forced on.
/// A custom OpenAI-compatible provider pointed at an Ollama server gets the
/// same probe from the endpoint auto-detection in `tinyinference-llm`.
fn ollama_limits_request(model: &str, config: &Config) -> Option<DiscoveryRequest> {
    let root = tinyinference_local::ollama::ollama_base_url_from_override(
        config.local_ai.base_url.as_deref(),
    );
    let model = model
        .split_once('@')
        .map_or(model, |(model, _)| model)
        .trim();
    if model.is_empty() {
        return None;
    }
    Some(
        DiscoveryRequest::new(format!("{}/v1", root.trim_end_matches('/')), model)
            .with_ollama_native(true),
    )
}

/// The fetcher discovery uses. Unit tests never reach the network.
fn default_fetcher(_config: &Config, _provider: &str) -> Box<dyn ModelListingFetcher> {
    #[cfg(test)]
    {
        Box::new(OfflineFetcher)
    }
    #[cfg(not(test))]
    {
        let provider_slug = _provider
            .split_once(':')
            .map_or(_provider, |(slug, _)| slug)
            .trim();
        let service_key = "inference.model_limits";
        let client = _config
            .cloud_provider_ca_certs
            .get(provider_slug)
            .filter(|pem| !pem.is_empty())
            .and_then(|pem| {
                crate::util::tls::client_with_ca_bundle_with_timeouts(pem, service_key, 5, 3).ok()
            })
            .unwrap_or_else(|| {
                crate::config::build_runtime_proxy_client_with_timeouts(service_key, 5, 3)
            });
        Box::new(tinyinference_llm::model::discover::ReqwestListingFetcher::new(client))
    }
}

/// A fetcher that never reaches the network (every request fails).
#[cfg(test)]
struct OfflineFetcher;

#[cfg(test)]
#[async_trait::async_trait]
impl ModelListingFetcher for OfflineFetcher {
    async fn get_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
    ) -> tinyinference_llm::Result<serde_json::Value> {
        Err(tinyinference_llm::Error::Catalog(format!(
            "offline in unit tests: {url}"
        )))
    }
}

/// Resolve the context window for `model` on the `role`'s `provider` route.
pub(crate) async fn resolve_context_window(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
) -> Option<u64> {
    let fetcher = default_fetcher(config, provider);
    resolve_context_window_with(
        fetcher.as_ref(),
        model_limits_cache(),
        role,
        provider,
        model,
        config,
    )
    .await
    .window
}

/// [`resolve_context_window`] with an explicit fetcher and cache.
pub(crate) async fn resolve_context_window_with(
    fetcher: &dyn ModelListingFetcher,
    cache: &ModelLimitsCache,
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
) -> ResolvedWindow {
    let local_kind = tinyinference_local::profile::kind_from_provider_string(provider);
    let request = match local_kind {
        None => {
            crate::inference::provider::factory::model_limits_request(role, provider, model, config)
        }
        Some(tinyinference_local::profile::LocalProviderKind::Ollama) => {
            ollama_limits_request(model, config)
        }
        Some(_) => None,
    };
    let static_window = super::model_context::static_context_window_for_model(model);
    let local_window = local_kind.and_then(|kind| {
        tinyinference_local::profile::context_window_with_local_fallback(
            model,
            static_window,
            Some(kind),
        )
    });
    let resolved = resolver()
        .resolve(
            provider,
            model,
            ContextWindowInput {
                config_override: config_override(model, config),
                discovery: request.as_ref(),
                static_window,
                has_local_profile: local_kind.is_some(),
                local_window,
            },
            fetcher,
            cache,
        )
        .await;
    if local_kind.is_none()
        && matches!(
            resolved.source,
            WindowSource::StaticGuess | WindowSource::Unknown
        )
    {
        warn_static_guess(model, resolved.window, provider);
    }
    if let (Some(window), Some(request)) = (resolved.window, request.as_ref()) {
        if matches!(
            resolved.source,
            WindowSource::ProviderReported | WindowSource::LearnedFromOverflow
        ) {
            tracing::info!(
                role,
                model = %request.model,
                endpoint = %request.endpoint,
                context_window = window,
                max_output_tokens = ?resolved.max_output_tokens,
                source = resolved.source.label(),
                "[model_context] context window from provider"
            );
        }
    }
    tracing::debug!(
        role,
        model,
        context_window = ?resolved.window,
        source = resolved.source.label(),
        "[model_context] resolved context window"
    );
    resolved
}

#[cfg(test)]
#[path = "context_window_tests.rs"]
mod tests;
