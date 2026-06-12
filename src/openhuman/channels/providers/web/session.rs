use serde_json::json;
use std::collections::HashSet;

use crate::openhuman::agent::profiles::{AgentProfile, DEFAULT_PROFILE_ID};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;

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

pub(super) fn pick_target_agent_id(_config: &Config, profile: &AgentProfile) -> String {
    if profile.id == DEFAULT_PROFILE_ID {
        "orchestrator".to_string()
    } else {
        profile.agent_id.clone()
    }
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
        Some("hint:reasoning") => "reasoning",
        _ => "chat",
    }
}

pub(super) fn build_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    target_agent_id: &str,
    profile: &AgentProfile,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<&str>,
) -> Result<Agent, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    let provider_role = provider_role_for_model_override(effective.default_model.as_deref());
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    log::info!(
        "[web-channel] routing chat turn to '{}' via profile '{}' provider_role='{}' (client_id={}, thread_id={})",
        target_agent_id,
        profile.id,
        provider_role,
        client_id,
        thread_id
    );

    let reflection_chunks = load_reflection_chunks_for_thread(&effective.workspace_dir, thread_id);

    if let Some(chunks) = reflection_chunks
        .as_ref()
        .filter(|chunks| !chunks.is_empty())
    {
        log::info!(
            "[web-channel] thread={} spawned from reflection — injecting {} memory chunks into system prompt",
            thread_id,
            chunks.len()
        );
    }

    let locale_directive = locale.and_then(locale_reply_directive);
    let composed_suffix = compose_system_prompt_suffix(
        locale_directive.as_deref(),
        profile.system_prompt_suffix.as_deref(),
    );
    if let Some(s) = locale_directive.as_deref() {
        log::info!(
            "[web-channel] injecting locale directive client={} thread={} locale={} directive={:?}",
            client_id,
            thread_id,
            locale.unwrap_or(""),
            s
        );
    }

    let agent_result = Agent::from_config_for_agent_with_profile(
        &effective,
        target_agent_id,
        reflection_chunks,
        composed_suffix,
    );

    agent_result
        .map(|mut agent| {
            if let Some(allowed_tools) = profile
                .allowed_tools
                .as_ref()
                .filter(|tools| !tools.is_empty())
            {
                agent.set_visible_tool_names(
                    allowed_tools
                        .iter()
                        .map(|tool| tool.trim().to_string())
                        .filter(|tool| !tool.is_empty())
                        .collect::<HashSet<_>>(),
                );
            }
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

fn load_reflection_chunks_for_thread(
    _workspace_dir: &std::path::Path,
    _thread_id: &str,
) -> Option<Vec<crate::openhuman::subconscious::SourceChunk>> {
    // Reflection store has been removed. Existing threads spawned from
    // reflections no longer receive memory-context injection.
    None
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

pub(crate) fn compose_system_prompt_suffix(
    locale_directive: Option<&str>,
    profile_suffix: Option<&str>,
) -> Option<String> {
    match (locale_directive, profile_suffix) {
        (None, None) => None,
        (Some(d), None) => Some(d.to_string()),
        (None, Some(p)) => Some(p.to_string()),
        (Some(d), Some(p)) => Some(format!("{d}\n\n{p}")),
    }
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
        provider_binding: crate::openhuman::inference::provider::provider_for_role(
            provider_role,
            config,
        ),
        target_agent_id,
        autonomy_signature: autonomy_signature(config),
        model_registry_signature: model_registry_signature(config),
    }
}
