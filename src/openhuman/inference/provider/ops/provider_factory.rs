use std::path::PathBuf;

use super::super::{reliable, router, traits::Provider};

/// Fixed id for the single inference backend (OpenHuman API).
pub const INFERENCE_BACKEND_ID: &str = "openhuman";

#[derive(Debug, Clone)]
pub struct ProviderRuntimeOptions {
    pub auth_profile_override: Option<String>,
    pub openhuman_dir: Option<PathBuf>,
    pub secrets_encrypt: bool,
    pub reasoning_enabled: Option<bool>,
}

impl Default for ProviderRuntimeOptions {
    fn default() -> Self {
        Self {
            auth_profile_override: None,
            openhuman_dir: None,
            secrets_encrypt: true,
            reasoning_enabled: None,
        }
    }
}

/// Create the inference provider.
///
/// - `inference_url`: optional custom OpenAI-compatible LLM endpoint
///   (`config.inference_url`). When set together with `api_key`, inference
///   talks directly to this URL — keeping product-backend traffic
///   (auth/billing/voice) on `backend_url` where it belongs.
/// - `backend_url`: the OpenHuman product backend URL (`config.api_url`).
///   Used by the fallback [`openhuman_backend::OpenHumanBackendProvider`]
///   which routes inference to `{backend}/openai/v1/...` with the app
///   session JWT.
/// - `api_key`: the API key for the custom inference endpoint. Ignored on
///   the OpenHuman fallback path (the backend uses a session JWT, not a
///   user-supplied key).
pub fn create_backend_inference_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    if let (Some(url), Some(key)) = (inference_url, api_key) {
        log::info!(
            "[providers] inference target = custom_openai @ {} (api_key bytes={})",
            url,
            key.len()
        );
        Ok(Box::new(
            crate::openhuman::inference::provider::compatible::OpenAiCompatibleProvider::new_no_responses_fallback(
                "custom_openai",
                url,
                Some(key),
                crate::openhuman::inference::provider::compatible::AuthStyle::Bearer,
            ),
        ))
    } else {
        if api_key.is_some() && inference_url.is_none() {
            log::warn!(
                "[providers] api_key provided without inference_url — key will be ignored, using OpenHuman backend"
            );
        }
        log::info!(
            "[providers] inference target = openhuman_backend (backend_url={}, inference_url_set={}, api_key_set={})",
            backend_url.unwrap_or("<default>"),
            inference_url.is_some(),
            api_key.is_some()
        );
        Ok(Box::new(
            crate::openhuman::inference::provider::openhuman_backend::OpenHumanBackendProvider::new(
                backend_url,
                options,
            ),
        ))
    }
}

/// Create provider chain with retry and fallback behavior.
pub fn create_resilient_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
) -> anyhow::Result<Box<dyn Provider>> {
    create_resilient_provider_with_options(
        inference_url,
        backend_url,
        api_key,
        reliability,
        &ProviderRuntimeOptions::default(),
    )
}

/// Create provider chain with retry/fallback behavior and auth runtime options.
pub fn create_resilient_provider_with_options(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    if !reliability.fallback_providers.is_empty() {
        tracing::warn!(
            "reliability.fallback_providers is ignored; inference uses only the OpenHuman backend"
        );
    }

    let primary_provider =
        create_backend_inference_provider(inference_url, backend_url, api_key, options)?;
    let providers: Vec<(String, Box<dyn Provider>)> =
        vec![(INFERENCE_BACKEND_ID.to_string(), primary_provider)];

    let reliable = reliable::ReliableProvider::new(
        providers,
        reliability.provider_retries,
        reliability.provider_backoff_ms,
    )
    .with_model_fallbacks(reliability.model_fallbacks.clone());

    Ok(Box::new(reliable))
}

/// Create a RouterProvider if model routes are configured, otherwise return a resilient provider.
pub fn create_routed_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
    model_routes: &[crate::openhuman::config::ModelRouteConfig],
    default_model: &str,
) -> anyhow::Result<Box<dyn Provider>> {
    create_routed_provider_with_options(
        inference_url,
        backend_url,
        api_key,
        reliability,
        model_routes,
        default_model,
        &ProviderRuntimeOptions::default(),
    )
}

pub fn create_routed_provider_with_options(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
    model_routes: &[crate::openhuman::config::ModelRouteConfig],
    default_model: &str,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    // Test-only: a mock provider injected by an e2e test wins over any
    // config-derived routing (covers the triage remote arm). Gated on
    // cfg(test) / the off-by-default `e2e-test-support` feature.
    #[cfg(any(test, feature = "e2e-test-support"))]
    if let Some(p) = super::super::factory::test_provider_override::current() {
        return Ok(Box::new(
            super::super::factory::test_provider_override::ProviderHandle(p),
        ));
    }

    if model_routes.is_empty() {
        return create_resilient_provider_with_options(
            inference_url,
            backend_url,
            api_key,
            reliability,
            options,
        );
    }

    let backend = create_backend_inference_provider(inference_url, backend_url, api_key, options)?;
    let providers: Vec<(String, Box<dyn Provider>)> =
        vec![(INFERENCE_BACKEND_ID.to_string(), backend)];

    let routes: Vec<(String, router::Route)> = model_routes
        .iter()
        .map(|r| {
            (
                r.hint.clone(),
                router::Route {
                    provider_name: INFERENCE_BACKEND_ID.to_string(),
                    model: r.model.clone(),
                    context_window:
                        crate::openhuman::inference::model_context::context_window_for_model(
                            &r.model,
                        ),
                },
            )
        })
        .collect();

    Ok(Box::new(router::RouterProvider::new(
        providers,
        routes,
        default_model.to_string(),
    )))
}

/// Create a provider with intelligent local/remote routing.
///
/// When `config.local_ai.runtime_enabled` is `true` and Ollama is reachable,
/// lightweight and medium tasks (e.g. `hint:reaction`, `hint:summarize`) are
/// served by the local model. Heavy tasks (`hint:reasoning`, `hint:agentic`,
/// `hint:coding`) always go to the remote backend. A health-gated fallback
/// transparently promotes failed local calls to the remote backend.
///
/// Telemetry for every routing decision is emitted at `INFO` level under the
/// `"routing"` tracing target.
pub fn create_intelligent_routing_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    config: &crate::openhuman::config::Config,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    let raw_backend =
        create_backend_inference_provider(inference_url, backend_url, api_key, options)?;
    // Wrap the raw backend in ReliableProvider so transient 502/503/504 errors
    // are retried before propagating to the agent turn. Without this, a single
    // 502 from the backend bypasses the retry layer entirely and surfaces as a
    // fatal `run_single` failure.
    log::debug!(
        "[providers] initialising reliable wrapper: retries={} backoff_ms={} fallbacks={}",
        config.reliability.provider_retries,
        config.reliability.provider_backoff_ms,
        config.reliability.model_fallbacks.len()
    );
    let reliable_backend: Box<dyn Provider> = Box::new(
        reliable::ReliableProvider::new(
            vec![(INFERENCE_BACKEND_ID.to_string(), raw_backend)],
            config.reliability.provider_retries,
            config.reliability.provider_backoff_ms,
        )
        .with_model_fallbacks(config.reliability.model_fallbacks.clone()),
    );
    let default_model = config
        .default_model
        .as_deref()
        .unwrap_or(crate::openhuman::config::DEFAULT_MODEL);

    // When the user has configured `model_routes` (custom provider via
    // BackendProviderPanel), wrap the reliable remote in a RouterProvider so
    // abstract tier names like `reasoning-v1` get translated to the configured
    // provider-specific model id (e.g. `gpt-5.5`) BEFORE the request leaves
    // the host. Without this step the abstract tier name would reach
    // `custom_openai` and 404. The OpenHuman backend can dispatch tier names
    // natively, so we skip the wrap when routes are empty.
    log::info!(
        "[providers] intelligent routing: model_routes_count={} default_model={} inference_url_set={}",
        config.model_routes.len(),
        default_model,
        inference_url.is_some()
    );
    let remote: Box<dyn Provider> = if config.model_routes.is_empty() {
        reliable_backend
    } else {
        let providers: Vec<(String, Box<dyn Provider>)> =
            vec![(INFERENCE_BACKEND_ID.to_string(), reliable_backend)];
        let routes: Vec<(String, router::Route)> = config
            .model_routes
            .iter()
            .map(|r| {
                (
                    r.hint.clone(),
                    router::Route {
                        provider_name: INFERENCE_BACKEND_ID.to_string(),
                        model: r.model.clone(),
                        context_window:
                            crate::openhuman::inference::model_context::context_window_for_model(
                                &r.model,
                            ),
                    },
                )
            })
            .collect();
        Box::new(router::RouterProvider::new(
            providers,
            routes,
            default_model.to_string(),
        ))
    };

    let provider = crate::openhuman::routing::new_provider(
        remote,
        &config.local_ai,
        default_model,
        &config.temperature_unsupported_models,
    );
    Ok(Box::new(provider))
}

/// Information about a supported provider for display purposes.
pub struct ProviderInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub local: bool,
}

/// Return known providers for display (single backend path).
pub fn list_providers() -> Vec<ProviderInfo> {
    vec![ProviderInfo {
        name: INFERENCE_BACKEND_ID,
        display_name: "OpenHuman (backend)",
        aliases: &["backend", "openhuman-backend"],
        local: false,
    }]
}

// Legacy provider alias stubs (integrations / config); remote providers were removed.
pub fn is_glm_alias(_name: &str) -> bool {
    false
}
pub fn is_zai_alias(_name: &str) -> bool {
    false
}
pub fn is_minimax_alias(_name: &str) -> bool {
    false
}
pub fn is_moonshot_alias(_name: &str) -> bool {
    false
}
pub fn is_qianfan_alias(_name: &str) -> bool {
    false
}
pub fn is_qwen_alias(_name: &str) -> bool {
    false
}
pub fn is_qwen_oauth_alias(_name: &str) -> bool {
    false
}
pub fn canonical_china_provider_name(_name: &str) -> Option<&'static str> {
    let _ = _name;
    None
}
