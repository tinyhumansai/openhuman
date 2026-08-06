//! Companion interaction pipeline: STT → LLM → TTS.
//!
//! Orchestrates a single interaction turn for the desktop companion. Migrated
//! from the former core `desktop_companion::pipeline`, re-plumbed for the shell:
//!
//! - **STT** and **TTS** are performed by the embedded core over JSON-RPC
//!   (`openhuman.voice_stt_dispatch` / `openhuman.voice_tts_dispatch`) via
//!   [`crate::core_rpc::call_core_rpc`].
//! - The **LLM turn** runs shell-side: it fetches the backend session token
//!   (`openhuman.auth_get_session_token`) and effective API URL
//!   (`openhuman.config_resolve_api_url`) from core, then POSTs
//!   `/openai/v1/chat/completions` directly with `reqwest`.
//! - Audio is captured natively by the shell (`super::audio`) and packed into a
//!   shell-local RIFF/WAVE container for upload (no dependency on the
//!   `meet`-gated `meet_agent::wav`).
//!
//! The pipeline is cancellable via [`tokio_util::sync::CancellationToken`] so
//! the shell can interrupt mid-turn (e.g. user presses the hotkey again during
//! Speaking).

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::session;
use super::types::*;

const LOG_PREFIX: &str = "[companion_pipeline]";

/// Maximum tokens for the companion LLM reply.
const REPLY_MAX_TOKENS: u32 = 512;

/// Rolling conversation context window (number of turns).
const CONTEXT_WINDOW: usize = 20;

/// Result of a single companion interaction turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResult {
    /// The user's transcribed speech (or typed input).
    pub transcript: String,
    /// The LLM's response text.
    pub response_text: String,
    /// Whether TTS audio was synthesized.
    pub tts_synthesized: bool,
    /// Whether the turn was cancelled before completion.
    pub cancelled: bool,
}

/// Run a text-input companion turn (no STT needed — the user typed their query).
///
/// **Precondition**: the session must already be in `Listening` state. The
/// caller is responsible for the `Idle → Listening` transition before invoking.
///
/// Transitions: Listening → Thinking → Speaking → Idle.
pub async fn run_text_turn(text: &str, cancel: CancellationToken) -> Result<TurnResult, String> {
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

    // LLM call.
    let response_text = match llm_companion(trimmed, &history_window).await {
        Ok(reply) => reply,
        Err(err) => {
            if cancel.is_cancelled() {
                return Ok(cancelled_result(trimmed));
            }
            warn!("{LOG_PREFIX} LLM failed err={err}");
            session::transition_state(CompanionState::Error, Some(format!("LLM failure: {err}")))?;
            return Err(format!("LLM failure: {err}"));
        }
    };

    if cancel.is_cancelled() {
        return Ok(cancelled_result(trimmed));
    }

    debug!("{LOG_PREFIX} LLM reply chars={}", response_text.len());

    // Record conversation turns.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let _ = session::push_conversation_turn(ConversationTurn {
        role: "user".into(),
        content: trimmed.to_string(),
        timestamp_ms: now_ms,
    });
    if !response_text.is_empty() {
        let _ = session::push_conversation_turn(ConversationTurn {
            role: "assistant".into(),
            content: response_text.clone(),
            timestamp_ms: now_ms,
        });
    }

    // TTS (skip if response is empty).
    let tts_ok = if !response_text.is_empty() && !cancel.is_cancelled() {
        session::transition_state(CompanionState::Speaking, None)?;
        match tts(&response_text).await {
            Ok(()) => {
                debug!("{LOG_PREFIX} TTS synthesized");
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

    // Back to idle.
    session::finish_turn();

    let result = TurnResult {
        transcript: trimmed.to_string(),
        response_text,
        tts_synthesized: tts_ok,
        cancelled: false,
    };

    info!("{LOG_PREFIX} text turn done");
    Ok(result)
}

/// Run a full audio-input companion turn: STT → LLM → TTS.
///
/// **Precondition**: the session must already be in `Listening` state.
///
/// Transitions: Listening → Thinking → Speaking → Idle.
pub async fn run_audio_turn(
    audio_samples: &[i16],
    sample_rate: u32,
    cancel: CancellationToken,
) -> Result<TurnResult, String> {
    if audio_samples.is_empty() {
        if cancel.is_cancelled() {
            return Ok(cancelled_result(""));
        }
        let message = "no audio samples".to_string();
        session::recover_after_turn_error(message.clone());
        return Err(message);
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
        Ok(text) if cancel.is_cancelled() => {
            return Ok(cancelled_result(text.trim()));
        }
        Ok(text) if text.trim().is_empty() => {
            info!("{LOG_PREFIX} STT returned empty transcript, skipping turn");
            super::finish_listening_without_active_capture();
            return Ok(TurnResult {
                transcript: String::new(),
                response_text: String::new(),
                tts_synthesized: false,
                cancelled: false,
            });
        }
        Ok(text) => text,
        Err(err) => {
            if cancel.is_cancelled() {
                return Ok(cancelled_result(""));
            }
            warn!("{LOG_PREFIX} STT failed err={err}");
            session::transition_state(CompanionState::Error, Some(format!("STT failure: {err}")))?;
            return Err(format!("STT failure: {err}"));
        }
    };

    debug!("{LOG_PREFIX} STT transcript chars={}", transcript.len());

    // Hand off to the text pipeline for the rest.
    run_text_turn(&transcript, cancel).await
}

// ─── Core-RPC / backend adapters ────────────────────────────────────

/// Transcribe audio samples through the core's configured STT provider.
///
/// The dispatch endpoint accepts base64 container bytes, so we pack the mono
/// PCM into a shell-local WAV container first.
async fn stt(samples: &[i16], sample_rate: u32) -> Result<String, String> {
    let wav = super::audio::pack_wav_pcm16le_mono(samples, sample_rate);
    let params = stt_dispatch_params(&wav);
    let result = crate::core_rpc::call_core_rpc("openhuman.voice_stt_dispatch", params).await?;
    debug!(
        "{LOG_PREFIX} stt provider={:?}",
        result.get("provider").and_then(|p| p.as_str())
    );
    result
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("unexpected transcribe response: {result}"))
}

fn stt_dispatch_params(wav: &[u8]) -> Value {
    json!({
        "audio_base64": BASE64_STANDARD.encode(wav),
        "mime_type": "audio/wav",
        "file_name": "companion.wav",
    })
}

/// Synthesize speech through the core's configured TTS provider.
///
/// The shell only needs to know synthesis succeeded to hold the Speaking
/// state; native playback remains a follow-up.
async fn tts(text: &str) -> Result<(), String> {
    let params = tts_dispatch_params(text);
    let result = crate::core_rpc::call_core_rpc("openhuman.voice_tts_dispatch", params).await?;
    debug!(
        "{LOG_PREFIX} tts audio_mime={:?}",
        result.get("audio_mime").and_then(|mime| mime.as_str())
    );
    Ok(())
}

fn tts_dispatch_params(text: &str) -> Value {
    json!({ "text": text })
}

/// Fetch the backend session token + effective API base URL from core so the
/// shell-side LLM turn can authenticate against `/openai/v1/chat/completions`.
async fn fetch_backend_creds() -> Result<(String, String), String> {
    let token_v = crate::core_rpc::call_core_rpc("openhuman.auth_get_session_token", json!({}))
        .await
        .map_err(|e| format!("fetch session token: {e}"))?;
    let token = token_v
        .get("token")
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "no backend session token".to_string())?
        .to_string();

    let url_v = crate::core_rpc::call_core_rpc("openhuman.config_resolve_api_url", json!({}))
        .await
        .map_err(|e| format!("resolve api url: {e}"))?;
    let backend_url = url_v
        .get("api_url")
        .and_then(|u| u.as_str())
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| "no backend api_url".to_string())?
        .to_string();

    Ok((token, backend_url))
}

/// System prompt for the desktop companion LLM.
const COMPANION_SYSTEM_PROMPT: &str = "\
You are OpenHuman, a helpful desktop AI companion. The user is talking to you \
via voice or text while using their computer.\n\
\n\
Guidelines:\n\
- Be concise and conversational. 1-3 sentences unless the question demands more.\n\
- Do not use markdown formatting (no asterisks, backticks, or bullet markers) — \
your response will be spoken aloud via TTS.\n\
- If you don't know or can't help, say so briefly.\n\
";

/// Build a chat-completions request with conversation history, then return the
/// assistant's reply. Runs shell-side against the
/// backend using creds fetched from core.
async fn llm_companion(prompt: &str, history: &[&ConversationTurn]) -> Result<String, String> {
    let (token, backend_url) = fetch_backend_creds().await?;
    let endpoint = companion_chat_endpoint(&backend_url);

    let mut messages: Vec<Value> = Vec::with_capacity(history.len() + 2);

    messages.push(json!({ "role": "system", "content": COMPANION_SYSTEM_PROMPT }));

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

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("chat completions request failed: {e}"))?;

    let status = resp.status();
    let raw: Value = resp
        .json()
        .await
        .map_err(|e| format!("chat completions: invalid JSON response: {e}"))?;

    if !status.is_success() {
        return Err(format!("chat completions returned {status}: {raw}"));
    }

    extract_chat_completion_text(&raw)
        .ok_or_else(|| format!("unexpected chat completions response: {raw}"))
}

fn companion_chat_endpoint(backend_url: &str) -> String {
    format!(
        "{}/openai/v1/chat/completions",
        backend_url.trim_end_matches('/')
    )
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
    session::finish_turn();
    info!("{LOG_PREFIX} turn cancelled, restored session to Idle");
    TurnResult {
        transcript: transcript.to_string(),
        response_text: String::new(),
        tts_synthesized: false,
        cancelled: true,
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
