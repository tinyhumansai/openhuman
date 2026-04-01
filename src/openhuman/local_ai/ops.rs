//! JSON-RPC / CLI controller surface for the bundled local AI stack.

use chrono::Utc;
use once_cell::sync::Lazy;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::local_ai::{
    self, LocalAiAssetsStatus, LocalAiDownloadsProgress, LocalAiEmbeddingResult,
    LocalAiSpeechResult, LocalAiTtsResult, Suggestion,
};
use crate::openhuman::providers::{self, ProviderRuntimeOptions};
use crate::rpc::RpcOutcome;

static REPL_AGENT_SESSIONS: Lazy<Mutex<HashMap<String, Agent>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn agent_chat(
    config: &mut Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<String>, String> {
    if let Some(model) = model_override {
        config.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        config.default_temperature = temp;
    }
    let mut agent = Agent::from_config(config).map_err(|e| e.to_string())?;
    let response = agent.run_single(message).await.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(response, "agent chat completed"))
}

pub async fn agent_chat_simple(
    config: &Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<String>, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    let default_model = effective
        .default_model
        .clone()
        .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string());

    let options = ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: effective.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: effective.secrets.encrypt,
        reasoning_enabled: effective.runtime.reasoning_enabled,
    };

    let provider = providers::create_routed_provider_with_options(
        effective.api_key.as_deref(),
        effective.api_url.as_deref(),
        &effective.reliability,
        &effective.model_routes,
        default_model.as_str(),
        &options,
    )
    .map_err(|e| e.to_string())?;

    let response = provider
        .chat_with_system(
            None,
            message,
            default_model.as_str(),
            effective.default_temperature,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(
        response,
        "agent simple chat completed",
    ))
}

pub async fn agent_repl_session_start(
    config: &Config,
    session_id: Option<String>,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    let mut requested = session_id.unwrap_or_default();
    requested = requested.trim().to_string();
    let session_id = if requested.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        requested
    };

    let agent = Agent::from_config(&effective).map_err(|e| e.to_string())?;
    REPL_AGENT_SESSIONS
        .lock()
        .await
        .insert(session_id.clone(), agent);

    Ok(RpcOutcome::single_log(
        json!({ "session_id": session_id }),
        "agent repl session started",
    ))
}

pub async fn agent_repl_session_reset(
    session_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("session_id is required".to_string());
    }

    let mut sessions = REPL_AGENT_SESSIONS.lock().await;
    let reset = if let Some(agent) = sessions.get_mut(session_id) {
        agent.clear_history();
        true
    } else {
        false
    };

    Ok(RpcOutcome::single_log(
        json!({ "reset": reset }),
        "agent repl session reset",
    ))
}

pub async fn agent_repl_session_end(
    session_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("session_id is required".to_string());
    }

    let ended = REPL_AGENT_SESSIONS
        .lock()
        .await
        .remove(session_id)
        .is_some();
    Ok(RpcOutcome::single_log(
        json!({ "ended": ended }),
        "agent repl session ended",
    ))
}

pub async fn local_ai_status(
    config: &Config,
) -> Result<RpcOutcome<local_ai::LocalAiStatus>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if matches!(status.state.as_str(), "idle" | "degraded") {
        let service_clone = service.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            service_clone.bootstrap(&config_clone).await;
        });
    }
    Ok(RpcOutcome::single_log(
        service.status(),
        "local ai status fetched",
    ))
}

pub async fn local_ai_download(
    config: &Config,
    force: bool,
) -> Result<RpcOutcome<local_ai::LocalAiStatus>, String> {
    let service = local_ai::global(config);
    if force {
        service.reset_to_idle(config);
    }
    let service_clone = service.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        if let Err(err) = service_clone.download_all_models(&config_clone).await {
            service_clone.mark_degraded(err);
        }
    });
    Ok(RpcOutcome::single_log(
        service.status(),
        "local ai full model download triggered",
    ))
}

pub async fn local_ai_download_all_assets(
    config: &Config,
    force: bool,
) -> Result<RpcOutcome<LocalAiDownloadsProgress>, String> {
    let service = local_ai::global(config);
    if force {
        service.reset_to_idle(config);
    }
    let service_clone = service.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        if let Err(err) = service_clone.download_all_models(&config_clone).await {
            service_clone.mark_degraded(err);
        }
    });
    let progress = service
        .downloads_progress(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        progress,
        "local ai full asset download triggered",
    ))
}

pub async fn local_ai_summarize(
    config: &Config,
    text: &str,
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let summary = service
        .summarize(config, text, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        summary,
        "local ai summarize completed",
    ))
}

pub async fn local_ai_suggest_questions(
    config: &Config,
    context: Option<String>,
    lines: Option<Vec<String>>,
) -> Result<RpcOutcome<Vec<Suggestion>>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let mut context = context.unwrap_or_default();
    if context.trim().is_empty() {
        if let Some(lines) = lines {
            context = lines.join("\n");
        }
    }
    let suggestions = service
        .suggest_questions(config, &context)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        suggestions,
        "local ai suggestions generated",
    ))
}

pub async fn local_ai_prompt(
    config: &Config,
    prompt: &str,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<RpcOutcome<String>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let output = service
        .prompt(config, prompt.trim(), max_tokens, no_think.unwrap_or(true))
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(output, "local ai prompt completed"))
}

pub async fn local_ai_vision_prompt(
    config: &Config,
    prompt: &str,
    image_refs: &[String],
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    let service = local_ai::global(config);
    let output = service
        .vision_prompt(config, prompt.trim(), image_refs, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai vision prompt completed",
    ))
}

pub async fn local_ai_embed(
    config: &Config,
    inputs: &[String],
) -> Result<RpcOutcome<LocalAiEmbeddingResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .embed(config, inputs)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai embedding completed",
    ))
}

pub async fn local_ai_transcribe(
    config: &Config,
    audio_path: &str,
) -> Result<RpcOutcome<LocalAiSpeechResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .transcribe(config, audio_path.trim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

pub async fn local_ai_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
) -> Result<RpcOutcome<LocalAiSpeechResult>, String> {
    let service = local_ai::global(config);

    let ext = extension
        .unwrap_or_else(|| "webm".to_string())
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Invalid audio extension".to_string());
    }

    let voice_dir = std::env::temp_dir().join("openhuman_voice_input");
    tokio::fs::create_dir_all(&voice_dir)
        .await
        .map_err(|e| format!("Failed to create voice input directory: {e}"))?;

    let filename = format!(
        "voice-{}-{}.{}",
        Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4(),
        ext
    );
    let file_path = voice_dir.join(filename);
    tokio::fs::write(&file_path, audio_bytes)
        .await
        .map_err(|e| format!("Failed to write audio file: {e}"))?;

    let output = service
        .transcribe(config, file_path.to_string_lossy().as_ref())
        .await;
    let _ = tokio::fs::remove_file(&file_path).await;

    let output = output.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

pub async fn local_ai_tts(
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<RpcOutcome<LocalAiTtsResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .tts(config, text.trim(), output_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(output, "local ai tts completed"))
}

pub async fn local_ai_assets_status(
    config: &Config,
) -> Result<RpcOutcome<LocalAiAssetsStatus>, String> {
    let service = local_ai::global(config);
    let output = service
        .assets_status(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai assets status fetched",
    ))
}

pub async fn local_ai_downloads_progress(
    config: &Config,
) -> Result<RpcOutcome<LocalAiDownloadsProgress>, String> {
    let service = local_ai::global(config);
    let output = service
        .downloads_progress(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai downloads progress fetched",
    ))
}

pub async fn local_ai_download_asset(
    config: &Config,
    capability: &str,
) -> Result<RpcOutcome<LocalAiAssetsStatus>, String> {
    let service = local_ai::global(config);
    let output = service
        .download_asset(config, capability.trim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai asset download triggered",
    ))
}

#[derive(Debug, serde::Deserialize)]
pub struct LocalAiChatMessage {
    pub role: String,
    pub content: String,
}

pub async fn local_ai_chat(
    config: &Config,
    messages: Vec<LocalAiChatMessage>,
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    tracing::debug!(
        message_count = messages.len(),
        "[local_ai:chat] local_ai_chat op: validating"
    );

    if messages.is_empty() {
        return Err("messages must not be empty".to_string());
    }

    let ollama_messages: Vec<crate::openhuman::local_ai::ollama_api::OllamaChatMessage> = messages
        .into_iter()
        .map(
            |m| crate::openhuman::local_ai::ollama_api::OllamaChatMessage {
                role: m.role,
                content: m.content,
            },
        )
        .collect();

    let service = local_ai::global(config);
    let reply = service
        .chat_with_history(config, ollama_messages, max_tokens)
        .await?;

    tracing::debug!(
        reply_len = reply.len(),
        "[local_ai:chat] local_ai_chat op: done"
    );
    Ok(RpcOutcome::single_log(reply, "local ai chat completed"))
}

/// Result of the reaction-decision prompt.
#[derive(Debug, serde::Serialize)]
pub struct ReactionDecision {
    /// Whether the model thinks a reaction is appropriate.
    pub should_react: bool,
    /// The emoji to use (only meaningful when `should_react` is true).
    pub emoji: Option<String>,
}

/// Ask the local model whether the assistant should add an emoji reaction to
/// the user's message, based on channel type and message content.
/// Designed to be called fire-and-forget — fast, lightweight, no cloud cost.
pub async fn local_ai_should_react(
    config: &Config,
    message: &str,
    channel_type: &str,
) -> Result<RpcOutcome<ReactionDecision>, String> {
    tracing::debug!(
        channel_type,
        msg_len = message.len(),
        "[local_ai:should_react] evaluating reaction"
    );

    if message.trim().is_empty() {
        return Ok(RpcOutcome::single_log(
            ReactionDecision {
                should_react: false,
                emoji: None,
            },
            "empty message — no reaction",
        ));
    }

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        tracing::debug!("[local_ai:should_react] local model not ready, skipping");
        return Ok(RpcOutcome::single_log(
            ReactionDecision {
                should_react: false,
                emoji: None,
            },
            "local model not ready",
        ));
    }

    let prompt = format!(
        "You decide whether an AI assistant should react to a user message with a single emoji. \
         Consider the channel context: casual channels (discord, telegram) get more frequent \
         reactions with playful emojis, while professional channels (web, slack, email) are more \
         reserved — only react to clearly emotional or noteworthy messages.\n\n\
         Channel: {channel_type}\nUser message: {message}\n\n\
         Reply with EXACTLY one word: either NONE (no reaction) or a single emoji character."
    );

    let output = service.prompt(config, &prompt, Some(8), true).await;

    let decision = match output {
        Ok(raw) => {
            let trimmed = raw.trim();
            tracing::debug!(
                output_len = trimmed.len(),
                "[local_ai:should_react] model response"
            );
            if trimmed.eq_ignore_ascii_case("NONE") || trimmed.is_empty() {
                ReactionDecision {
                    should_react: false,
                    emoji: None,
                }
            } else {
                // Extract the first emoji-like character(s) from the response
                let emoji = extract_first_emoji(trimmed);
                match emoji {
                    Some(e) => ReactionDecision {
                        should_react: true,
                        emoji: Some(e),
                    },
                    None => ReactionDecision {
                        should_react: false,
                        emoji: None,
                    },
                }
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "[local_ai:should_react] inference failed, skipping");
            ReactionDecision {
                should_react: false,
                emoji: None,
            }
        }
    };

    tracing::debug!(
        should_react = decision.should_react,
        emoji = ?decision.emoji,
        "[local_ai:should_react] decision"
    );
    Ok(RpcOutcome::single_log(
        decision,
        "reaction decision completed",
    ))
}

/// Extract the first emoji from a string. Handles common emoji codepoints
/// including flag sequences (pairs of regional indicator symbols).
fn extract_first_emoji(text: &str) -> Option<String> {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        // Regional indicator pair → flag emoji (e.g. 🇺🇸 = U+1F1FA U+1F1F8)
        if is_regional_indicator(ch) {
            let mut emoji = String::new();
            emoji.push(ch);
            // Consume consecutive regional indicators (flags are pairs)
            for next in chars.by_ref() {
                if is_regional_indicator(next) {
                    emoji.push(next);
                } else {
                    break;
                }
            }
            return Some(emoji);
        }

        if is_emoji_start(ch) {
            let mut emoji = String::new();
            emoji.push(ch);
            // Consume joiners and variation selectors that extend the emoji
            for next in chars.by_ref() {
                if next == '\u{FE0F}'     // variation selector
                    || next == '\u{200D}'  // zero-width joiner
                    || ('\u{1F3FB}'..='\u{1F3FF}').contains(&next) // skin tones
                    || is_emoji_start(next) && emoji.contains('\u{200D}')
                {
                    emoji.push(next);
                } else {
                    break;
                }
            }
            return Some(emoji);
        }
    }
    None
}

fn is_regional_indicator(ch: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch)
}

fn is_emoji_start(ch: char) -> bool {
    matches!(ch,
        '\u{203C}' | '\u{2049}'       // exclamation marks
        | '\u{2139}'                   // information
        | '\u{2194}'..='\u{2199}'      // arrows
        | '\u{21A9}'..='\u{21AA}'      // arrows
        | '\u{231A}'..='\u{231B}'      // watch, hourglass
        | '\u{23E9}'..='\u{23F3}'      // media controls
        | '\u{23F8}'..='\u{23FA}'      // media controls
        | '\u{24C2}'                   // circled M
        | '\u{25AA}'..='\u{25AB}'      // squares
        | '\u{25B6}' | '\u{25C0}'     // play buttons
        | '\u{25FB}'..='\u{25FE}'      // squares
        | '\u{2328}' | '\u{23CF}'     // keyboard, eject
        | '\u{2600}'..='\u{27BF}'      // misc symbols, dingbats
        | '\u{2934}'..='\u{2935}'      // arrows
        | '\u{2B05}'..='\u{2B07}'      // arrows
        | '\u{2B1B}'..='\u{2B1C}'      // squares
        | '\u{2B50}' | '\u{2B55}'     // star, circle
        | '\u{FE00}'..='\u{FE0F}'      // variation selectors
        | '\u{1F300}'..='\u{1F9FF}'    // misc symbols, emoticons, transport, supplemental
        | '\u{1FA00}'..='\u{1FA6F}'    // chess symbols, extended-A
        | '\u{1FA70}'..='\u{1FAFF}'    // symbols extended-A
        | '\u{200D}'                   // ZWJ
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_emoji_from_simple_string() {
        assert_eq!(extract_first_emoji("👍"), Some("👍".to_string()));
        assert_eq!(extract_first_emoji("🔥"), Some("🔥".to_string()));
        assert_eq!(extract_first_emoji("❤️"), Some("❤️".to_string()));
    }

    #[test]
    fn extract_emoji_with_surrounding_text() {
        assert_eq!(extract_first_emoji("Sure! 😂"), Some("😂".to_string()));
        assert_eq!(
            extract_first_emoji("I think 👀 fits here"),
            Some("👀".to_string())
        );
    }

    #[test]
    fn extract_none_when_no_emoji() {
        assert_eq!(extract_first_emoji("NONE"), None);
        assert_eq!(extract_first_emoji("no reaction"), None);
        assert_eq!(extract_first_emoji(""), None);
    }

    #[test]
    fn extract_flag_emoji_keeps_pair_together() {
        assert_eq!(
            extract_first_emoji("🇺🇸"),
            Some("🇺🇸".to_string())
        );
        assert_eq!(
            extract_first_emoji("🇬🇧 Great Britain"),
            Some("🇬🇧".to_string())
        );
    }

    #[test]
    fn is_emoji_start_recognizes_common_emojis() {
        assert!(is_emoji_start('👍'));
        assert!(is_emoji_start('🔥'));
        assert!(is_emoji_start('😂'));
        assert!(is_emoji_start('⭐'));
        assert!(!is_emoji_start('A'));
        assert!(!is_emoji_start('1'));
    }
}
