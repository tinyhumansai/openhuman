use std::sync::Arc;
use std::time::Duration;

use crate::openhuman::config::LocalAiConfig;
use crate::openhuman::local_ai::ollama_base_url;
use crate::openhuman::providers::compatible::{AuthStyle, OpenAiCompatibleProvider};
use crate::openhuman::providers::Provider;

use super::health::LocalHealthChecker;
use super::provider::IntelligentRoutingProvider;

/// Cache TTL for the non-ollama local health probe. Mirrors the default used
/// by [`LocalHealthChecker::new`].
const LOCAL_HEALTH_TTL: Duration = Duration::from_secs(30);

/// Construct an [`IntelligentRoutingProvider`] from a remote backend provider
/// and the local AI configuration.
///
/// When `local_ai_config.enabled` is `false` the returned provider behaves
/// identically to the remote provider (local health always returns `false`).
///
/// `remote_fallback_model` is the model string sent to the remote backend when
/// a lightweight/medium task falls back from a failed local call. Typically
/// this is the configured `default_model` (e.g. `"reasoning-v1"`).
pub fn new_provider(
    remote: Box<dyn Provider>,
    local_ai_config: &LocalAiConfig,
    remote_fallback_model: &str,
) -> IntelligentRoutingProvider {
    // Allow operators to point the local routing tier at an OpenAI-compatible
    // server other than Ollama (e.g. llama-server for Gemma 4 E2B, which
    // Ollama's embedded llama.cpp cannot load yet as of April 2026).
    //
    // `OPENHUMAN_LOCAL_INFERENCE_URL` — full `/v1` base URL of the local
    // OpenAI-compat server. When set, health is probed via `GET {base}/models`
    // instead of Ollama's `/api/tags`.
    let override_base = std::env::var("OPENHUMAN_LOCAL_INFERENCE_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());

    let provider_kind = local_ai_config.provider.trim().to_ascii_lowercase();
    let use_openai_compat_local = override_base.is_some()
        || matches!(
            provider_kind.as_str(),
            "llamacpp" | "llama-server" | "custom_openai"
        );

    let (provider_label, local_base, health) = if use_openai_compat_local {
        let base = override_base
            .or_else(|| local_ai_config.base_url.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8080/v1".to_string());
        let probe = format!("{base}/models");
        tracing::debug!(
            provider = %provider_kind,
            "[routing] local inference configured via OpenAI-compat (non-ollama)"
        );
        (
            if provider_kind == "custom_openai" {
                "custom_openai"
            } else {
                "llamacpp"
            },
            base,
            Arc::new(LocalHealthChecker::with_probe_url(probe, LOCAL_HEALTH_TTL)),
        )
    } else {
        let ollama_base = ollama_base_url();
        let local_v1 = format!("{ollama_base}/v1");
        (
            "ollama",
            local_v1,
            Arc::new(LocalHealthChecker::new(&ollama_base)),
        )
    };

    let local: Box<dyn Provider> = Box::new(OpenAiCompatibleProvider::new(
        provider_label,
        &local_base,
        local_ai_config.api_key.as_deref(),
        AuthStyle::Bearer,
    ));

    IntelligentRoutingProvider::new(
        remote,
        local,
        local_ai_config.chat_model_id.clone(),
        remote_fallback_model.to_string(),
        local_ai_config.enabled,
        health,
    )
}
