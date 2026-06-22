//! JSON-RPC / CLI controller surface for the bundled local AI stack.
//!
//! This module provides high-level functions for interacting with local AI
//! services such as agent chat, model downloads, summarization, and
//! transcription. These functions are typically invoked via RPC or CLI.

use chrono::Utc;

use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::inference::local as local_ai;
use crate::openhuman::inference::provider as providers;
use crate::openhuman::inference::provider::ops::ProviderRuntimeOptions;
use crate::openhuman::inference::{
    LocalAiAssetsStatus, LocalAiDownloadsProgress, LocalAiEmbeddingResult, LocalAiSpeechResult,
    LocalAiStatus, LocalAiTtsResult,
};
use crate::openhuman::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::rpc::RpcOutcome;

fn prompt_guard_user_message(action: PromptEnforcementAction) -> &'static str {
    match action {
        PromptEnforcementAction::Allow => "Message accepted.",
        PromptEnforcementAction::Blocked => {
            "Prompt blocked by security policy. Please rephrase without instruction overrides or exfiltration requests."
        }
        PromptEnforcementAction::ReviewBlocked => {
            "Prompt flagged for security review and was not processed. Please rephrase clearly."
        }
    }
}

/// Normalize a `model_override` string into the `Option<String>` form the
/// downstream config-resolution path expects.
///
/// `None` → `None`. `Some(non-empty-after-trim)` → `Some(trimmed)`. Anything
/// else (`Some("")`, `Some("   ")`, `Some("\t\n")`) collapses to `None` so
/// the existing default-model fallback applies instead of overwriting
/// `config.default_model` with a blank string that the OpenHuman backend
/// would reject with `400 model is required` (Sentry TAURI-RUST-RS).
///
/// Extracted to keep `agent_chat` and `agent_chat_simple` in lockstep —
/// future tweaks (additional log lines, tightening the trim rules) live in
/// exactly one place.
fn normalize_model_override(opt: Option<String>) -> Option<String> {
    opt.and_then(|m| {
        let t = m.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

fn enforce_user_prompt_or_reject(prompt: &str, source: &'static str) -> Result<(), String> {
    let decision = enforce_prompt_input(
        prompt,
        PromptEnforcementContext {
            source,
            request_id: None,
            user_id: None,
            session_id: Some("local_ai"),
        },
    );
    match decision.action {
        PromptEnforcementAction::Allow => Ok(()),
        PromptEnforcementAction::Blocked | PromptEnforcementAction::ReviewBlocked => {
            Err(prompt_guard_user_message(decision.action).to_string())
        }
    }
}

/// Executes a single chat turn with an AI agent.
///
/// This function initializes an agent from the provided configuration and
/// processes the input message.
///
/// # Arguments
///
/// * `config` - The configuration used to build the agent. May be updated with model/temp overrides.
/// * `message` - The user message to process.
/// * `model_override` - Optional model name to use for this call.
/// * `temperature` - Optional sampling temperature override.
pub async fn agent_chat(
    config: &mut Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(message, "local_ai.ops.agent_chat")?;

    // TAURI-RUST-RS: an upstream caller (frontend, JSON-RPC client) can pass
    // `model_override: Some("")`. See `normalize_model_override` for the
    // rationale — an empty / whitespace-only override collapses to `None`.
    if let Some(model) = normalize_model_override(model_override) {
        config.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        config.default_temperature = temp;
    }
    let mut agent = Agent::from_config(config).map_err(|e| e.to_string())?;
    // Direct `agent_chat` RPC — invoked by trusted clients (desktop UI,
    // operator CLI). Label as CLI so the approval gate doesn't fail
    // closed on an unlabelled call site.
    let run = crate::openhuman::agent::turn_origin::with_origin(
        crate::openhuman::agent::turn_origin::AgentTurnOrigin::Cli,
        agent.run_single(message),
    );
    let response = match thread_id.as_deref() {
        Some(id) if !id.trim().is_empty() => {
            log::debug!("[inference] agent_chat routing with thread_id={id}");
            crate::openhuman::inference::provider::thread_context::with_thread_id(id, run).await
        }
        _ => {
            log::debug!("[inference] agent_chat routing without thread_id");
            run.await
        }
    }
    .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(response, "agent chat completed"))
}

/// A simplified chat interface that does not update the base configuration.
pub async fn agent_chat_simple(
    config: &Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(message, "local_ai.ops.agent_chat_simple")?;

    let mut effective = config.clone();
    // TAURI-RUST-RS: see `normalize_model_override` for the rationale.
    if let Some(model) = normalize_model_override(model_override) {
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
        config.inference_url.as_deref(),
        config.api_url.as_deref(),
        config.api_key.as_deref(),
        &effective.reliability,
        &effective.model_routes,
        default_model.as_str(),
        &options,
    )
    .map_err(|e| e.to_string())?;

    let run = provider.chat_with_system(
        None,
        message,
        default_model.as_str(),
        effective.default_temperature,
    );
    let response = match thread_id.as_deref() {
        Some(id) if !id.trim().is_empty() => {
            log::debug!("[inference] agent_chat_simple routing with thread_id={id}");
            crate::openhuman::inference::provider::thread_context::with_thread_id(id, run).await
        }
        _ => {
            log::debug!("[inference] agent_chat_simple routing without thread_id");
            run.await
        }
    }
    .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(
        response,
        "agent simple chat completed",
    ))
}

/// Returns the current operational status of the local AI stack.
pub async fn local_ai_status(config: &Config) -> Result<RpcOutcome<LocalAiStatus>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if matches!(status.state.as_str(), "idle" | "degraded") {
        let service_clone = service.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            service_clone.bootstrap(&config_clone).await;
        });
    }
    // `LocalAiService` is a process-wide singleton whose cached `provider`
    // field was set at first init from whichever config it saw. After an
    // `inference_update_local_settings` call that swaps providers
    // (e.g. ollama → lm_studio) the cached value is stale, so we overlay
    // the current config's provider on the status snapshot before returning.
    let mut snapshot = service.status();
    snapshot.provider = local_ai::provider::provider_from_config(config)
        .as_str()
        .to_string();
    Ok(RpcOutcome::single_log(snapshot, "local ai status fetched"))
}

/// Generates a summary of the provided text using local AI models.
pub async fn local_ai_summarize(
    config: &Config,
    text: &str,
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(text.trim(), "local_ai.ops.local_ai_summarize")?;

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let summary = service
        .summarize_interactive(config, text, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        summary,
        "local ai summarize completed",
    ))
}

/// Executes a raw prompt directly against the local AI model.
pub async fn local_ai_prompt(
    config: &Config,
    prompt: &str,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(prompt.trim(), "local_ai.ops.local_ai_prompt")?;

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let output = service
        .prompt_interactive(config, prompt.trim(), max_tokens, no_think.unwrap_or(true))
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(output, "local ai prompt completed"))
}

/// Executes a multimodal (vision) prompt with associated images.
pub async fn local_ai_vision_prompt(
    config: &Config,
    prompt: &str,
    image_refs: &[String],
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(prompt.trim(), "local_ai.ops.local_ai_vision_prompt")?;

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

/// Generates semantic embeddings for the provided input strings.
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

/// Transcribes the audio file at the specified path.
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

/// Transcribes raw audio bytes by first saving them to a temporary file.
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

/// Performs text-to-speech synthesis and optionally saves the result to a file.
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

/// Returns the status of all local AI assets (models and support files).
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

/// Returns progress for any ongoing asset downloads.
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

/// Triggers the download of a specific AI asset based on capability name.
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

/// A single message in a local AI chat conversation.
#[derive(Debug, serde::Deserialize)]
pub struct LocalAiChatMessage {
    /// The role of the message sender (e.g., "user", "assistant").
    pub role: String,
    /// The text content of the message.
    pub content: String,
}

/// Executes a multi-turn chat conversation using the local model.
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

    let mut ollama_messages: Vec<crate::openhuman::inference::local::ollama::OllamaChatMessage> =
        Vec::with_capacity(messages.len());

    for msg in messages.into_iter() {
        let normalized_role = msg.role.trim().to_ascii_lowercase();
        match normalized_role.as_str() {
            "user" => {
                enforce_user_prompt_or_reject(msg.content.as_str(), "local_ai.ops.local_ai_chat")?;
            }
            "system" | "assistant" => {}
            _ => {
                return Err(format!(
                    "unsupported message role: '{}'; expected one of: user, system, assistant",
                    msg.role.trim()
                ));
            }
        }

        ollama_messages.push(
            crate::openhuman::inference::local::ollama::OllamaChatMessage {
                role: normalized_role,
                content: msg.content,
            },
        );
    }

    let service = local_ai::global(config);
    let reply = service
        .chat_with_history_interactive(config, ollama_messages, max_tokens)
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

/// Evaluates whether the assistant should add an emoji reaction to a user message.
///
/// This uses the local model to make a quick decision based on the message
/// content and the channel context.
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
#[path = "ops_tests.rs"]
mod tests;
