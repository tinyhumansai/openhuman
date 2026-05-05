//! Cloud speech-to-text — proxies the hosted backend's
//! `/openai/v1/audio/transcriptions` endpoint so the desktop UI can transcribe
//! mic input without shipping a provider API key. Mirrors the shape of
//! `reply_speech.rs`, but uploads multipart form data instead of JSON.
//!
//! Used by the mascot's mic-only composer (`HumanPage`) — recording is
//! captured via `MediaRecorder` in the renderer, base64-encoded, then sent
//! through this RPC. The transcribed text is fed straight into the agent's
//! existing send pipeline.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use log::debug;
use reqwest::header::AUTHORIZATION;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice_cloud_stt]";

/// Default model id sent to the backend. The backend's controller currently
/// resolves this to whichever provider it has configured for audio
/// transcription (today: GMI Whisper). Callers can override.
const DEFAULT_MODEL: &str = "whisper-v1";

/// Caller-tunable knobs.
#[derive(Debug, Default, Clone)]
pub struct CloudTranscribeOptions {
    pub model: Option<String>,
    pub language: Option<String>,
    pub mime_type: Option<String>,
    /// Original file name hint (e.g. `audio.webm`). Some upstream providers
    /// sniff the extension; without one we fall back to `audio.webm`.
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTranscribeResult {
    pub text: String,
}

/// Decode + upload audio bytes to the backend STT endpoint.
///
/// `audio_base64` is what comes off the wire from the renderer — keeping the
/// UI side base64 means we don't have to reach for a binary RPC channel.
pub async fn transcribe_cloud(
    config: &Config,
    audio_base64: &str,
    opts: &CloudTranscribeOptions,
) -> Result<RpcOutcome<CloudTranscribeResult>, String> {
    let trimmed = audio_base64.trim();
    if trimmed.is_empty() {
        return Err("audio_base64 is required".to_string());
    }
    let audio_bytes = BASE64
        .decode(trimmed)
        .map_err(|e| format!("invalid base64 audio: {e}"))?;
    if audio_bytes.is_empty() {
        return Err("decoded audio is empty".to_string());
    }

    let token = get_session_token(config)
        .map_err(|e| e.to_string())?
        .and_then(|t| {
            let s = t.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
        .ok_or_else(|| "no backend session token; sign in first".to_string())?;

    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let url = client
        .url_for("/openai/v1/audio/transcriptions")
        .map_err(|e| e.to_string())?;

    let mime = opts
        .mime_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("audio/webm")
        .to_string();
    let file_name = opts
        .file_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("audio.webm")
        .to_string();
    let model = opts
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MODEL)
        .to_string();

    let bytes_len = audio_bytes.len();
    let part = Part::bytes(audio_bytes)
        .file_name(file_name.clone())
        .mime_str(&mime)
        .map_err(|e| format!("invalid mime '{mime}': {e}"))?;

    let mut form = Form::new().part("file", part).text("model", model.clone());
    if let Some(lang) = opts
        .language
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        form = form.text("language", lang.to_string());
    }

    debug!(
        "{LOG_PREFIX} POST {} mime={} bytes={} model={}",
        url.path(),
        mime,
        bytes_len,
        model
    );

    let upload_started = std::time::Instant::now();
    let response = client
        .raw_client()
        .post(url.clone())
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("backend transcription request failed: {e}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read transcription response failed: {e}"))?;
    let upload_ms = upload_started.elapsed().as_millis();
    debug!(
        "{LOG_PREFIX} backend responded status={} upload_round_trip_ms={} body_bytes={}",
        status,
        upload_ms,
        body.len()
    );
    if !status.is_success() {
        return Err(format!(
            "POST /openai/v1/audio/transcriptions failed ({status}): {body}"
        ));
    }

    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("parse transcription response failed: {e}; body={body}"))?;
    // A 200 with no string `text` field is a backend contract break — surface
    // it as an error rather than swallowing it as a successful empty
    // transcription, which would look to the caller like "no speech detected".
    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("transcription response missing string `text`: {body}"))?
        .trim()
        .to_string();

    debug!("{LOG_PREFIX} transcribed chars={}", text.len());

    Ok(RpcOutcome::single_log(
        CloudTranscribeResult { text },
        "cloud STT via POST /openai/v1/audio/transcriptions",
    ))
}
