//! Local AI provider selection helpers.

use crate::openhuman::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalAiProvider {
    Ollama,
    LmStudio,
}

impl LocalAiProvider {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lm_studio",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
        }
    }
}

pub(crate) fn normalize_provider(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "lmstudio" | "lm-studio" | "lm_studio" => LocalAiProvider::LmStudio.as_str().to_string(),
        // OMLX is a keyed OpenAI-v1 local runtime handled by the provider factory
        // (`omlx:<model>`), not by `LocalAiProvider`. Preserve the slug so the
        // saved config keeps `provider = "omlx"` instead of collapsing to ollama.
        "omlx" | "omlx-server" => {
            log::trace!(
                "[local-provider] normalized provider '{}' -> omlx (factory-resolved local runtime)",
                value.trim()
            );
            "omlx".to_string()
        }
        _ => LocalAiProvider::Ollama.as_str().to_string(),
    }
}

pub(crate) fn provider_from_config(config: &Config) -> LocalAiProvider {
    match normalize_provider(&config.local_ai.provider).as_str() {
        "lm_studio" => LocalAiProvider::LmStudio,
        _ => LocalAiProvider::Ollama,
    }
}

/// How a local runtime exposes its installed-model catalog.
///
/// Ollama serves a native `GET /api/tags` listing; every OpenAI-compatible
/// runtime (LM Studio, OMLX, `local-openai`, and any custom BYOK endpoint that
/// speaks the OpenAI `/v1` surface) serves `GET /v1/models`. Sending an Ollama
/// probe to an OpenAI-compatible server produces `GET /v1/api/tags`, which
/// LM Studio logs as `Unexpected endpoint or method` and answers with an empty
/// catalog — so model discovery silently fails and the model never appears as
/// selectable (GH #5053).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelDiscoveryApi {
    /// Ollama-native `/api/tags`.
    OllamaTags,
    /// OpenAI-compatible `/v1/models`.
    OpenAiModels,
}

/// True when `base_url`'s path is the OpenAI-compatible `/v1` root — the
/// canonical marker of an OpenAI-style server (LM Studio, OMLX, LiteLLM, …).
///
/// A genuine Ollama base URL is host-rooted (`http://localhost:11434`) with no
/// `/v1` path segment, so this cleanly separates the two API shapes by
/// **endpoint type**, not by "is it localhost" (which would misidentify a
/// localhost LM Studio server as Ollama — the #5053 conflation).
pub(crate) fn endpoint_is_openai_v1(base_url: &str) -> bool {
    if let Ok(url) = reqwest::Url::parse(base_url.trim()) {
        let path = url.path().trim_end_matches('/').to_ascii_lowercase();
        return path.ends_with("/v1");
    }
    // Fall back to a lexical check when the URL doesn't parse cleanly.
    base_url
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
        .ends_with("/v1")
}

/// Select the model-discovery API for a local runtime from its provider slug
/// and base URL — by provider **type**, never by "is it localhost".
///
/// Genuine Ollama (`provider = "ollama"` on a host-rooted base) uses
/// `/api/tags`. Every OpenAI-compatible runtime uses `/v1/models`: an explicit
/// `lm_studio` / `omlx` slug, OR any endpoint whose path is the OpenAI `/v1`
/// root. The `/v1` endpoint clause is what rescues a custom BYOK localhost
/// endpoint (e.g. LM Studio on `http://localhost:1234/v1`) whose provider tag
/// still defaults to `ollama`: the endpoint type wins over the fallback slug
/// (GH #5053).
pub(crate) fn model_discovery_api(provider: &str, base_url: &str) -> ModelDiscoveryApi {
    if endpoint_is_openai_v1(base_url) {
        return ModelDiscoveryApi::OpenAiModels;
    }
    match normalize_provider(provider).as_str() {
        "ollama" => ModelDiscoveryApi::OllamaTags,
        // lm_studio, omlx, and any other OpenAI-compatible local runtime.
        _ => ModelDiscoveryApi::OpenAiModels,
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
