//! OpenHuman's own inference, served to the module's `Complete` callback.
//!
//! Every call goes through the provider OpenHuman is configured with for the
//! `summarization` role (managed backend, BYOK cloud, or local), the same
//! path thread titles and follow-up suggestions use. Reasoning is switched off
//! for these calls only; a rewrite needs none, and reasoning models otherwise
//! spend ~10 s per call. The rest of OpenHuman keeps its configured reasoning.

use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelRequest;

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::inference::provider;

/// Workload role whose provider tinymemes borrows.
pub(crate) const ROLE: &str = "summarization";
const TEMPERATURE: f64 = 0.7;

/// One system + user completion on the `summarization` provider.
pub(crate) async fn complete(
    ctx: &DispatchCtx,
    config: &Config,
    system: &str,
    user: &str,
) -> Result<String, String> {
    // Built per call so a provider change in Settings applies without a
    // restart; construction is cheap and synchronous.
    let (model, resolved) =
        provider::create_chat_model_with_model_id_in(ctx, ROLE, config, TEMPERATURE)
            .map_err(|e| format!("openhuman inference unavailable: {e}"))?;
    // Reasoning off for tinymemes' calls only (the rest of OpenHuman keeps
    // its configured reasoning). The managed backend honours the
    // `without_reasoning` flag; a BYOK route to OpenRouter needs
    // OpenRouter's own switch. Other endpoints are left untouched, since an
    // unknown field can be rejected.
    let mut request = provider::openhuman_backend_model::without_reasoning(
        ModelRequest::new(vec![Message::system(system), Message::user(user)])
            .with_temperature(TEMPERATURE),
    );
    if routes_to_openrouter(config) {
        request.provider_options = serde_json::json!({ "reasoning": { "enabled": false } });
    }
    let response = model
        .invoke(&(), request)
        .await
        .map_err(|e| format!("openhuman inference failed model={resolved}: {e}"))?;
    let text = response.text();
    if text.trim().is_empty() {
        return Err(format!(
            "openhuman inference returned no text model={resolved}"
        ));
    }
    Ok(text)
}

/// Whether tinymemes' role resolves to an OpenRouter endpoint: a BYOK
/// `openrouter:` route, or a custom inference URL on openrouter.ai.
pub(crate) fn routes_to_openrouter(config: &Config) -> bool {
    let route_is_openrouter = config
        .memory_provider
        .as_deref()
        .map(|p| p.trim().to_ascii_lowercase())
        .is_some_and(|p| p.starts_with("openrouter:") || p == "openrouter");
    let url_is_openrouter = config
        .inference_url
        .as_deref()
        .is_some_and(|u| u.to_ascii_lowercase().contains("openrouter.ai"));
    route_is_openrouter || url_is_openrouter
}

/// The model id OpenHuman would use for tinymemes, for the startup log line.
pub(crate) fn resolved_model(ctx: &DispatchCtx, config: &Config) -> String {
    provider::create_chat_model_with_model_id_in(ctx, ROLE, config, TEMPERATURE)
        .map(|(_, id)| id)
        .unwrap_or_else(|e| format!("unavailable ({e})"))
}

#[cfg(test)]
#[path = "inference_tests.rs"]
mod tests;
