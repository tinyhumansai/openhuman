//! Companion interaction pipeline: STT → screen context → LLM → TTS → pointing.
//!
//! Orchestrates a single interaction turn for the desktop companion. Reuses
//! the same cloud STT, LLM, and TTS backends that `meet_agent::brain` uses,
//! but adds screenshot + foreground-app context and POINT-tag parsing for
//! visual pointing.
//!
//! The pipeline is cancellable via [`tokio_util::sync::CancellationToken`] so
//! the Tauri shell can interrupt mid-turn (e.g. user presses hotkey again
//! during Speaking).

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::handoff::{self, HandoffEvent};
use super::pointing::{self, PointTarget, PointingParseResult, ScreenGeometry};
use super::session;
use super::types::*;

const LOG_PREFIX: &str = "[companion_pipeline]";

/// Maximum tokens for the companion LLM reply. Longer than meet_agent (220)
/// because the companion can give richer answers when not constrained to
/// live-meeting brevity.
const REPLY_MAX_TOKENS: u32 = 512;

/// Rolling conversation context window (number of turns).
const CONTEXT_WINDOW: usize = 20;

/// ElevenLabs TTS model for companion speech.
const TTS_MODEL_ID: &str = "eleven_turbo_v2_5";

/// Result of a single companion interaction turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResult {
    /// The user's transcribed speech (or typed input).
    pub transcript: String,
    /// The LLM's response text (POINT tags stripped).
    pub response_text: String,
    /// Parsed pointing targets from the LLM response.
    pub targets: Vec<PointTarget>,
    /// Whether TTS audio was synthesized and enqueued.
    pub tts_synthesized: bool,
    /// Provider-surface handoff events detected in the response.
    pub handoff_events: Vec<HandoffEvent>,
    /// Whether the turn was cancelled before completion.
    pub cancelled: bool,
}

/// Run a text-input companion turn (no STT needed — the user typed their query).
///
/// **Precondition**: the session must already be in `Listening` state. The
/// caller (e.g. Tauri hotkey bridge or `companion_activate` RPC) is
/// responsible for the `Idle → Listening` transition before invoking this.
///
/// Transitions: Listening → Thinking → Speaking/Pointing → Idle.
pub async fn run_text_turn(
    text: &str,
    screens: &[ScreenGeometry],
    cancel: CancellationToken,
) -> Result<TurnResult, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("empty input".into());
    }

    info!("{LOG_PREFIX} text turn start chars={}", trimmed.len());

    // Transition to Thinking.
    session::transition_state(CompanionState::Thinking, None)?;

    if cancel.is_cancelled() {
        return Ok(cancelled_result(trimmed));
    }

    // Gather conversation history.
    let history = session::conversation_history();
    let history_window = tail_history(&history, CONTEXT_WINDOW);

    // Screen context (best-effort — skip if unavailable).
    let screen_context = gather_screen_context().await;

    if cancel.is_cancelled() {
        return Ok(cancelled_result(trimmed));
    }

    // LLM call.
    let raw_reply = match llm_companion(trimmed, &history_window, screen_context.as_deref()).await {
        Ok(reply) => reply,
        Err(err) => {
            warn!("{LOG_PREFIX} LLM failed err={err}");
            session::transition_state(CompanionState::Error, Some(format!("LLM failure: {err}")))?;
            return Err(format!("LLM failure: {err}"));
        }
    };

    if cancel.is_cancelled() {
        return Ok(cancelled_result(trimmed));
    }

    // Parse POINT tags.
    let PointingParseResult {
        targets,
        clean_text,
    } = pointing::parse_and_map(&raw_reply, screens);

    debug!(
        "{LOG_PREFIX} LLM reply chars={} targets={}",
        clean_text.len(),
        targets.len()
    );

    // Check for provider-surface handoff opportunities.
    let handoff_events = handoff::check_handoff(&clean_text);
    if !handoff_events.is_empty() {
        debug!("{LOG_PREFIX} handoff events={}", handoff_events.len());
    }

    // Record conversation turns.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let _ = session::push_conversation_turn(ConversationTurn {
        role: "user".into(),
        content: trimmed.to_string(),
        timestamp_ms: now_ms,
    });
    if !clean_text.is_empty() {
        let _ = session::push_conversation_turn(ConversationTurn {
            role: "assistant".into(),
            content: clean_text.clone(),
            timestamp_ms: now_ms,
        });
    }

    // TTS (skip if response is empty).
    let tts_ok = if !clean_text.is_empty() && !cancel.is_cancelled() {
        session::transition_state(CompanionState::Speaking, None)?;
        match tts(&clean_text).await {
            Ok(_samples) => {
                debug!("{LOG_PREFIX} TTS synthesized samples={}", _samples.len());
                true
            }
            Err(err) => {
                warn!("{LOG_PREFIX} TTS failed err={err} (continuing without audio)");
                false
            }
        }
    } else {
        false
    };

    if cancel.is_cancelled() {
        return Ok(cancelled_result(trimmed));
    }

    // Pointing phase.
    if !targets.is_empty() {
        let _ = session::transition_state(CompanionState::Pointing, None);
    }

    // Back to idle.
    let _ = session::transition_state(CompanionState::Idle, None);

    let result = TurnResult {
        transcript: trimmed.to_string(),
        response_text: clean_text,
        targets,
        tts_synthesized: tts_ok,
        handoff_events,
        cancelled: false,
    };

    info!("{LOG_PREFIX} text turn done");
    Ok(result)
}

/// Run a full audio-input companion turn: STT → screen context → LLM → TTS → pointing.
///
/// **Precondition**: the session must already be in `Listening` state. The
/// caller (e.g. Tauri hotkey bridge or `companion_activate` RPC) is
/// responsible for the `Idle → Listening` transition before invoking this.
///
/// Transitions: Listening → Thinking → Speaking/Pointing → Idle.
pub async fn run_audio_turn(
    audio_samples: &[i16],
    sample_rate: u32,
    screens: &[ScreenGeometry],
    cancel: CancellationToken,
) -> Result<TurnResult, String> {
    if audio_samples.is_empty() {
        return Err("no audio samples".into());
    }

    info!(
        "{LOG_PREFIX} audio turn start samples={} rate={sample_rate}",
        audio_samples.len()
    );

    // Check cancellation before expensive STT call.
    if cancel.is_cancelled() {
        return Ok(cancelled_result(""));
    }

    // STT.
    let transcript = match stt(audio_samples, sample_rate).await {
        Ok(text) if text.trim().is_empty() => {
            info!("{LOG_PREFIX} STT returned empty transcript, skipping turn");
            let _ = session::transition_state(CompanionState::Idle, None);
            return Ok(TurnResult {
                transcript: String::new(),
                response_text: String::new(),
                targets: Vec::new(),
                tts_synthesized: false,
                handoff_events: Vec::new(),
                cancelled: false,
            });
        }
        Ok(text) => text,
        Err(err) => {
            warn!("{LOG_PREFIX} STT failed err={err}");
            session::transition_state(CompanionState::Error, Some(format!("STT failure: {err}")))?;
            return Err(format!("STT failure: {err}"));
        }
    };

    debug!("{LOG_PREFIX} STT transcript chars={}", transcript.len());

    // Hand off to the text pipeline for the rest.
    run_text_turn(&transcript, screens, cancel).await
}

// ─── Real adapters ──────────────────────────────────────────────────

/// Transcribe audio samples to text via cloud STT.
async fn stt(samples: &[i16], sample_rate: u32) -> Result<String, String> {
    use crate::openhuman::voice::cloud_transcribe::{transcribe_cloud, CloudTranscribeOptions};

    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    let wav_bytes = crate::openhuman::meet_agent::wav::pack_pcm16le_mono_wav(samples, sample_rate);
    let audio_b64 = B64.encode(&wav_bytes);
    let opts = CloudTranscribeOptions {
        mime_type: Some("audio/wav".to_string()),
        file_name: Some("companion.wav".to_string()),
        ..Default::default()
    };
    let outcome = transcribe_cloud(&config, &audio_b64, &opts).await?;
    Ok(outcome.value.text.clone())
}

/// System prompt for the desktop companion LLM.
const COMPANION_SYSTEM_PROMPT: &str = "\
You are OpenHuman, a helpful desktop AI companion. The user is talking to you \
via voice or text while using their computer. You can see their screen \
(a screenshot and foreground app info may be provided).\n\
\n\
Guidelines:\n\
- Be concise and conversational. 1-3 sentences unless the question demands more.\n\
- When you want to point the user to a UI element on screen, embed a \
`[POINT:x,y:label:screenN]` tag in your response where x,y are pixel \
coordinates relative to the screen, label describes the element, and N is \
the zero-based screen index.\n\
- Do not use markdown formatting (no asterisks, backticks, or bullet markers) — \
your response will be spoken aloud via TTS.\n\
- If screen context is provided, reference what you see when relevant.\n\
- If you don't know or can't help, say so briefly.\n\
";

/// Build a chat-completions request with screen context and conversation
/// history, then return the assistant's reply.
async fn llm_companion(
    prompt: &str,
    history: &[&ConversationTurn],
    screen_context: Option<&str>,
) -> Result<String, String> {
    use crate::api::config::effective_backend_api_url;
    use crate::api::jwt::get_session_token;
    use crate::api::BackendOAuthClient;
    use reqwest::Method;

    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    let token = get_session_token(&config)
        .map_err(|e| e.to_string())?
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "no backend session token".to_string())?;

    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;

    let mut messages: Vec<Value> = Vec::with_capacity(history.len() + 3);

    // System prompt with optional screen context.
    let system = if let Some(ctx) = screen_context {
        format!(
            "{COMPANION_SYSTEM_PROMPT}\n\
             Current screen context:\n{ctx}"
        )
    } else {
        COMPANION_SYSTEM_PROMPT.to_string()
    };
    messages.push(json!({ "role": "system", "content": system }));

    // Rolling conversation history.
    for turn in history {
        messages.push(json!({ "role": turn.role, "content": turn.content }));
    }

    // Current user message.
    messages.push(json!({ "role": "user", "content": prompt }));

    let body = json!({
        "model": "agentic-v1",
        "temperature": 0.5,
        "max_tokens": REPLY_MAX_TOKENS,
        "messages": messages,
    });

    // `flatten_authed_error` maps the typed `BackendApiError::Unauthorized`
    // (expected session-lapse 401) onto the `SESSION_EXPIRED` sentinel so the
    // JSON-RPC layer classifies it as session expiry and skips Sentry, matching
    // the #3384 team/billing pattern and the voice TTS fix (TAURI-RUST-8X1).
    // The previous `e.to_string()` leaked every lapsed-session companion-voice
    // 401 to Sentry as a hard error; every other error keeps its `{e:#}` chain.
    let raw = client
        .authed_json(
            &token,
            Method::POST,
            "/openai/v1/chat/completions",
            Some(body),
        )
        .await
        .map_err(crate::api::flatten_authed_error)?;

    extract_chat_completion_text(&raw)
        .ok_or_else(|| format!("unexpected chat completions response: {raw}"))
}

/// Synthesize speech from text via cloud TTS. Returns raw PCM16LE samples.
async fn tts(text: &str) -> Result<Vec<i16>, String> {
    use crate::openhuman::voice::reply_speech::{synthesize_reply, ReplySpeechOptions};

    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    let voice_settings = json!({
        "stability": 0.4,
        "similarity_boost": 0.75,
        "style": 0.35,
        "use_speaker_boost": true,
    });
    let opts = ReplySpeechOptions {
        output_format: Some("pcm_16000".to_string()),
        model_id: Some(TTS_MODEL_ID.to_string()),
        voice_settings: Some(voice_settings),
        ..Default::default()
    };
    let outcome = synthesize_reply(&config, text, &opts).await?;
    let pcm_bytes = B64
        .decode(outcome.value.audio_base64.as_bytes())
        .map_err(|e| format!("decode tts base64: {e}"))?;
    if pcm_bytes.len() % 2 != 0 {
        return Err(format!("odd byte length from tts: {}", pcm_bytes.len()));
    }
    Ok(pcm_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect())
}

/// Gather screen context (foreground app info) as a text summary.
/// Returns `None` if screen intelligence is unavailable.
async fn gather_screen_context() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let context = crate::openhuman::accessibility::foreground_context();
        context.map(|ctx| {
            format!(
                "App: {} | Window: {}",
                ctx.app_name.as_deref().unwrap_or("unknown"),
                ctx.window_title.as_deref().unwrap_or("unknown"),
            )
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn extract_chat_completion_text(raw: &Value) -> Option<String> {
    raw.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
}

/// Take the last `n` turns from conversation history.
fn tail_history(history: &[ConversationTurn], n: usize) -> Vec<&ConversationTurn> {
    let start = history.len().saturating_sub(n);
    history[start..].iter().collect()
}

fn cancelled_result(transcript: &str) -> TurnResult {
    // Restore session to Idle so it doesn't stay stuck in Thinking/Speaking.
    let _ = session::transition_state(CompanionState::Idle, None);
    info!("{LOG_PREFIX} turn cancelled, restored session to Idle");
    TurnResult {
        transcript: transcript.to_string(),
        response_text: String::new(),
        targets: Vec::new(),
        tts_synthesized: false,
        handoff_events: Vec::new(),
        cancelled: true,
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
