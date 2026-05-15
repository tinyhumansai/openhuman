use std::sync::Arc;
use std::time::Duration;

use crate::openhuman::config::LocalAiConfig;
use crate::openhuman::local_ai::lm_studio_api::lm_studio_base_url_from_local_ai;
use crate::openhuman::local_ai::ollama_base_url;
use crate::openhuman::local_ai::provider::normalize_provider;
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
/// When `local_ai_config.runtime_enabled` is `false` the returned provider behaves
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

    // Resolve the provider string: use the canonical helper for LM Studio
    // aliases ("lm-studio", "lmstudio" → "lm_studio"), but preserve other
    // provider strings ("llamacpp", "llama-server", "custom_openai") as-is so
    // their own branches below still match.
    let provider_kind = local_ai_config.provider.trim().to_ascii_lowercase();
    let local_provider_kind: String = {
        let normalized = normalize_provider(&provider_kind);
        if normalized == "lm_studio" {
            normalized
        } else {
            provider_kind.clone()
        }
    };
    let use_openai_compat_local = override_base.is_some()
        || matches!(
            local_provider_kind.as_str(),
            "lm_studio" | "llamacpp" | "llama-server" | "custom_openai"
        );

    let (provider_label, local_base, health) = if local_provider_kind == "lm_studio" {
        let base = override_base
            .clone()
            .unwrap_or_else(|| lm_studio_base_url_from_local_ai(local_ai_config));
        let probe = format!("{base}/models");
        tracing::debug!(
            provider = %local_provider_kind,
            base = %base,
            "[routing] local inference configured via LM Studio"
        );
        (
            "lm_studio",
            base,
            Arc::new(LocalHealthChecker::with_probe_url(probe, LOCAL_HEALTH_TTL)),
        )
    } else if use_openai_compat_local {
        let base = override_base
            .clone()
            .or_else(|| local_ai_config.base_url.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8080/v1".to_string());
        let probe = format!("{base}/models");
        tracing::debug!(
            provider = %local_provider_kind,
            "[routing] local inference configured via OpenAI-compat (non-ollama)"
        );
        (
            if local_provider_kind.as_str() == "custom_openai" {
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

    let local_api_key = local_ai_config
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let local_auth_style = if local_api_key.is_some() {
        AuthStyle::Bearer
    } else {
        AuthStyle::None
    };
    let local: Box<dyn Provider> = Box::new(OpenAiCompatibleProvider::new(
        provider_label,
        &local_base,
        local_api_key,
        local_auth_style,
    ));

    IntelligentRoutingProvider::new(
        remote,
        local,
        local_ai_config.chat_model_id.clone(),
        remote_fallback_model.to_string(),
        local_ai_config.runtime_enabled,
        health,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::LocalAiConfig;
    use crate::openhuman::providers::traits::{ProviderCapabilities, ToolsPayload};
    use crate::openhuman::tools::ToolSpec;
    use async_trait::async_trait;

    struct StubProvider;

    #[async_trait]
    impl Provider for StubProvider {
        async fn chat_with_system(
            &self,
            _system: Option<&str>,
            _msg: &str,
            _model: &str,
            _temp: f64,
        ) -> anyhow::Result<String> {
            Ok("stub".to_string())
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                native_tool_calling: false,
                vision: false,
            }
        }
        fn convert_tools(&self, _tools: &[ToolSpec]) -> ToolsPayload {
            ToolsPayload::PromptGuided {
                instructions: String::new(),
            }
        }
    }

    fn make_provider(config: &LocalAiConfig) -> IntelligentRoutingProvider {
        new_provider(Box::new(StubProvider), config, "remote-fallback")
    }

    /// Test that construction does not panic and the provider is usable.
    /// Private fields are not readable from outside the module, so we verify
    /// via observable behaviour (supports_streaming, capabilities).
    #[test]
    fn factory_local_disabled_when_runtime_disabled_does_not_support_local_streaming() {
        let mut cfg = LocalAiConfig::default();
        cfg.runtime_enabled = false;
        let p = make_provider(&cfg);
        // When local is disabled, the routing provider defers everything to
        // remote. StubProvider reports `supports_streaming = false`, so the
        // composite must surface that — this also exercises the
        // local-disabled branch in supports_streaming without panicking.
        assert!(
            !p.supports_streaming(),
            "expected remote streaming capability (StubProvider=false) when local runtime is disabled"
        );
    }

    #[test]
    fn factory_constructs_without_panic_when_runtime_enabled() {
        let mut cfg = LocalAiConfig::default();
        cfg.runtime_enabled = true;
        cfg.chat_model_id = "gemma3:4b-it-qat".to_string();
        let _p = make_provider(&cfg);
    }

    #[test]
    fn factory_llamacpp_provider_constructs_without_panic() {
        // When provider is "llamacpp" the health probe URL must be
        // `{base}/models` (OpenAI-compat), not `{base}/api/tags` (Ollama).
        // We verify construction does not panic and the routing provider
        // is usable.
        let mut cfg = LocalAiConfig::default();
        cfg.runtime_enabled = true;
        cfg.provider = "llamacpp".to_string();
        cfg.base_url = Some("http://127.0.0.1:8080/v1".to_string());
        let _p = make_provider(&cfg);
    }

    #[test]
    fn factory_custom_openai_provider_constructs_without_panic() {
        let mut cfg = LocalAiConfig::default();
        cfg.runtime_enabled = true;
        cfg.provider = "custom_openai".to_string();
        cfg.base_url = Some("http://127.0.0.1:1234/v1".to_string());
        let _p = make_provider(&cfg);
    }

    #[test]
    fn factory_lm_studio_provider_constructs_without_panic() {
        let mut cfg = LocalAiConfig::default();
        cfg.runtime_enabled = true;
        cfg.provider = "lm-studio".to_string();
        cfg.base_url = Some("http://127.0.0.1:1234/v1".to_string());
        cfg.chat_model_id = "local-model".to_string();
        let _p = make_provider(&cfg);
    }

    #[test]
    fn factory_llama_server_alias_is_recognised() {
        // "llama-server" is an alias for the llamacpp OpenAI-compat path.
        let mut cfg = LocalAiConfig::default();
        cfg.runtime_enabled = true;
        cfg.provider = "llama-server".to_string();
        cfg.base_url = Some("http://127.0.0.1:8080/v1".to_string());
        let _p = make_provider(&cfg);
    }

    #[test]
    fn factory_env_override_url_takes_precedence_over_base_url() {
        // OPENHUMAN_LOCAL_INFERENCE_URL env var must override config.base_url.
        // This is tested by ensuring construction succeeds when the env var
        // is set — a real URL check would require a running server.
        let _guard = crate::openhuman::local_ai::local_ai_test_guard();
        unsafe {
            std::env::set_var("OPENHUMAN_LOCAL_INFERENCE_URL", "http://127.0.0.1:9999/v1");
        }
        let mut cfg = LocalAiConfig::default();
        cfg.runtime_enabled = true;
        cfg.base_url = Some("http://should-be-ignored:1234/v1".to_string());
        // Should construct without panic — env override is recognised.
        let _p = make_provider(&cfg);
        unsafe {
            std::env::remove_var("OPENHUMAN_LOCAL_INFERENCE_URL");
        }
    }
}
