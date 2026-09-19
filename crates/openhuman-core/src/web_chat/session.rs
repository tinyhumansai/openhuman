//! Builds and fingerprints the cached session `Agent`: target agent
//! resolution, model-override normalization, locale reply directive, and the
//! `SessionCacheFingerprint` that decides whether a cached agent can be
//! reused for the next turn on a thread.

use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use serde_json::json;

use super::types::SessionCacheFingerprint;

pub(super) fn autonomy_signature(config: &Config) -> String {
    serde_json::to_string(&config.autonomy).unwrap_or_default()
}

/// Signature of `config.model_registry` for the session-cache fingerprint.
/// Captures every per-model `vision` flag so toggling one in Settings forces a
/// rebuild (picking up the new build-time `model_vision`). Mirrors
/// [`autonomy_signature`].
pub(super) fn model_registry_signature(config: &Config) -> String {
    serde_json::to_string(&config.model_registry).unwrap_or_default()
}

pub(super) fn pick_target_agent_id(_config: &Config) -> String {
    "orchestrator".to_string()
}

pub(crate) fn normalize_model_override(model_override: Option<String>) -> Option<String> {
    model_override
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

pub(crate) fn provider_role_for_model_override(model_override: Option<&str>) -> &'static str {
    match model_override.map(str::trim) {
        Some("hint:agentic") | Some("agentic-v1") => "agentic",
        Some("hint:coding") | Some("coding-v1") => "coding",
        Some("hint:summarization") | Some("summarization-v1") => "summarization",
        Some("hint:reasoning") | Some("reasoning-v1") => "reasoning",
        _ => "chat",
    }
}

pub(super) fn build_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    target_agent_id: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<&str>,
) -> Result<OpenHumanSessionHost, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    let provider_role = provider_role_for_model_override(effective.default_model.as_deref());
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    log::info!(
        "[web-channel] routing chat turn to '{}' provider_role='{}' (client_id={}, thread_id={})",
        target_agent_id,
        provider_role,
        client_id,
        thread_id
    );

    let locale_directive = locale.and_then(locale_reply_directive);
    if let Some(s) = locale_directive.as_deref() {
        log::info!(
            "[web-channel] injecting locale directive client={} thread={} locale={} directive={:?}",
            client_id,
            thread_id,
            locale.unwrap_or(""),
            s
        );
    }

    let agent_result = OpenHumanSessionHost::from_config_for_agent(&effective, target_agent_id);

    agent_result
        .map(|mut agent| {
            agent.set_event_context(
                json!({"client_id": client_id, "thread_id": thread_id}).to_string(),
                "web_channel",
            );
            let short_thread = if thread_id.len() > 12 {
                &thread_id[..12]
            } else {
                thread_id
            };
            agent.set_agent_definition_name(format!("{target_agent_id}_{short_thread}"));
            agent
        })
        .map_err(|e| e.to_string())
}

pub(crate) fn locale_reply_directive(locale: &str) -> Option<String> {
    let language = match locale.trim() {
        "ar" => "Arabic",
        "bn" => "Bengali",
        "es" => "Spanish",
        "fr" => "French",
        "hi" => "Hindi",
        "id" => "Indonesian",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "zh-CN" | "zh" => "Simplified Chinese",
        _ => return None,
    };
    Some(format!(
        "User language: the user's interface is set to {language}. \
         Respond in {language} unless the user explicitly asks for a different language. \
         Keep proper nouns, code, and command names untranslated."
    ))
}

pub(super) fn build_session_fingerprint(
    config: &Config,
    model_override: Option<String>,
    temperature: Option<f64>,
    target_agent_id: String,
    provider_role: &str,
) -> SessionCacheFingerprint {
    SessionCacheFingerprint {
        model_override,
        temperature,
        provider_binding: crate::inference::provider::provider_for_role(provider_role, config),
        target_agent_id,
        autonomy_signature: autonomy_signature(config),
        model_registry_signature: model_registry_signature(config),
    }
}
