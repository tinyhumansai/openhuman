use std::sync::Arc;

use crate::openhuman::config::LocalAiConfig;
use crate::openhuman::local_ai::OLLAMA_BASE_URL;
use crate::openhuman::providers::compatible::{AuthStyle, OpenAiCompatibleProvider};
use crate::openhuman::providers::Provider;

use super::health::LocalHealthChecker;
use super::provider::IntelligentRoutingProvider;

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
    let local_base = format!("{}/v1", OLLAMA_BASE_URL);
    let local: Box<dyn Provider> = Box::new(OpenAiCompatibleProvider::new(
        "ollama",
        &local_base,
        None, // Ollama does not require authentication
        AuthStyle::Bearer,
    ));

    let health = Arc::new(LocalHealthChecker::new(OLLAMA_BASE_URL));

    IntelligentRoutingProvider::new(
        remote,
        local,
        local_ai_config.chat_model_id.clone(),
        remote_fallback_model.to_string(),
        local_ai_config.enabled,
        health,
    )
}
