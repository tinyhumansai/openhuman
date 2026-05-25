//! Factory functions for creating voice (STT / TTS) providers.
//!
//! Mirrors the shape of [`crate::openhuman::embeddings::factory`]: a single
//! entry point that takes a provider name + parameters and returns a boxed
//! trait object. Production paths pick the provider based on the user's
//! config (`stt_provider`, `tts_provider`); unit tests use the factory
//! directly to verify dispatch branches.
//!
//! ## Provider-string grammar
//!
//! Mirrors the LLM inference factory pattern in
//! [`crate::openhuman::inference::provider::factory`]:
//!
//! | String                | Resolves to                                    |
//! |-----------------------|------------------------------------------------|
//! | `"cloud"` / `"openhuman"` | OpenHuman backend proxy                    |
//! | `"whisper"`           | Local Whisper (STT)                            |
//! | `"piper"`             | Local Piper (TTS)                              |
//! | `"<slug>:<model>"`    | Voice provider entry matched by slug           |
//! | `"<slug>"`            | Bare slug — uses provider's default model/voice|
//!
//! ## STT providers
//!
//! - `"cloud"` → backend Whisper proxy (POST `/openai/v1/audio/transcriptions`).
//! - `"whisper"` → local Whisper via `WHISPER_BIN` (or in-process `whisper-rs`).
//! - `"<slug>:<model>"` → third-party STT API via the voice provider registry
//!   (e.g. `"deepgram:nova-2"`, `"openai:whisper-1"`).
//!
//! ## TTS providers
//!
//! - `"cloud"` → backend ElevenLabs proxy (POST `/openai/v1/audio/speech`)
//!   which also returns Oculus-15 visemes for the mascot lip-sync.
//! - `"piper"` → local Piper subprocess via `PIPER_BIN`.
//! - `"<slug>:<voice>"` → third-party TTS API via the voice provider registry
//!   (e.g. `"openai:alloy"`, `"elevenlabs:<voice_id>"`).
//!
//! ## Logging prefixes
//!
//! All factory branches log against `[voice-factory]`; the wrapped provider
//! implementations log under `[voice-stt]` / `[voice-tts]` so end-to-end
//! traces grep cleanly.

use std::sync::Arc;

use async_trait::async_trait;
use log::debug;
use serde::{Deserialize, Serialize};

use super::cloud_transcribe::{transcribe_cloud, CloudTranscribeOptions, CloudTranscribeResult};
use super::local_speech::{synthesize_piper, PiperOptions};
use super::local_transcribe::{transcribe_whisper, WhisperTranscribeOptions};
use super::reply_speech::{synthesize_reply, ReplySpeechOptions, ReplySpeechResult};
use crate::openhuman::config::schema::voice_providers::{
    SttApiStyle, TtsApiStyle, VoiceCapability,
};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice-factory]";

// ---------------------------------------------------------------------------
// Provider traits
// ---------------------------------------------------------------------------

/// Common shape both STT branches return after dispatch. Keeps the wire
/// contract identical regardless of provider — the UI only sees `text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttResult {
    pub text: String,
    /// Lowercase provider id (`"cloud"`, `"whisper"`) — exposed on the wire
    /// so the renderer can show the user which path actually ran.
    pub provider: String,
}

/// Speech-to-text provider abstraction. Cloud (backend proxy) and Whisper
/// (local subprocess / in-process) both implement this; the factory hands
/// the caller a boxed trait object.
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Stable identifier used in logs and config (`"cloud"`, `"whisper"`).
    fn name(&self) -> &'static str;

    /// Transcribe a single base64-encoded audio blob.
    ///
    /// `mime_type` and `file_name` are hints; providers that don't care
    /// may ignore them. `language` is BCP-47 (`"en"`, `"es"`); pass `None`
    /// to let the provider auto-detect.
    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String>;
}

/// Text-to-speech provider abstraction. Cloud returns rich viseme alignment
/// (used by the mascot lip-sync); Piper returns audio only and the caller
/// derives a flat viseme timeline downstream.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Synthesize speech for `text`. Returns the same envelope shape as
    /// `voice.reply_synthesize` so the renderer can swap providers without
    /// branching on the response.
    async fn synthesize(
        &self,
        config: &Config,
        text: &str,
        voice: Option<&str>,
    ) -> Result<RpcOutcome<ReplySpeechResult>, String>;
}

// ---------------------------------------------------------------------------
// Cloud STT
// ---------------------------------------------------------------------------

/// Cloud STT — wraps [`transcribe_cloud`]. Stateless; cheap to construct.
pub struct CloudSttProvider {
    model: String,
}

impl CloudSttProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }
}

#[async_trait]
impl SttProvider for CloudSttProvider {
    fn name(&self) -> &'static str {
        "cloud"
    }

    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String> {
        debug!(
            "{LOG_PREFIX} cloud STT dispatch model={} bytes_b64={}",
            self.model,
            audio_base64.len()
        );
        let opts = CloudTranscribeOptions {
            model: Some(self.model.clone()),
            language: language.map(str::to_string),
            mime_type: mime_type.map(str::to_string),
            file_name: file_name.map(str::to_string),
        };
        let outcome = transcribe_cloud(config, audio_base64, &opts).await?;
        let CloudTranscribeResult { text } = outcome.value;
        Ok(RpcOutcome::single_log(
            SttResult {
                text,
                provider: "cloud".to_string(),
            },
            "voice-factory: cloud STT completed",
        ))
    }
}

// ---------------------------------------------------------------------------
// Local Whisper STT
// ---------------------------------------------------------------------------

/// Local Whisper STT — wraps [`transcribe_whisper`]. Resolves `WHISPER_BIN`
/// lazily on each call.
pub struct WhisperSttProvider {
    model: String,
}

impl WhisperSttProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }
}

#[async_trait]
impl SttProvider for WhisperSttProvider {
    fn name(&self) -> &'static str {
        "whisper"
    }

    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        _file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String> {
        debug!(
            "{LOG_PREFIX} whisper STT dispatch model={} mime={:?} lang={:?}",
            self.model, mime_type, language
        );
        let opts = WhisperTranscribeOptions {
            model: Some(self.model.clone()),
            mime_type: mime_type.map(str::to_string),
            language: language.map(str::to_string),
        };
        let outcome = transcribe_whisper(config, audio_base64, &opts).await?;
        Ok(RpcOutcome::single_log(
            SttResult {
                text: outcome.value.text,
                provider: "whisper".to_string(),
            },
            "voice-factory: whisper STT completed",
        ))
    }
}

// ---------------------------------------------------------------------------
// Cloud TTS
// ---------------------------------------------------------------------------

/// Cloud TTS — wraps [`synthesize_reply`] (backend ElevenLabs proxy).
pub struct CloudTtsProvider {
    voice: Option<String>,
}

impl CloudTtsProvider {
    pub fn new(voice: Option<String>) -> Self {
        Self { voice }
    }
}

#[async_trait]
impl TtsProvider for CloudTtsProvider {
    fn name(&self) -> &'static str {
        "cloud"
    }

    async fn synthesize(
        &self,
        config: &Config,
        text: &str,
        voice: Option<&str>,
    ) -> Result<RpcOutcome<ReplySpeechResult>, String> {
        let resolved_voice = voice
            .map(str::to_string)
            .or_else(|| self.voice.clone())
            .filter(|s| !s.trim().is_empty());
        debug!(
            "{LOG_PREFIX} cloud TTS dispatch voice={} chars={}",
            resolved_voice.as_deref().unwrap_or("<default>"),
            text.len()
        );
        let opts = ReplySpeechOptions {
            voice_id: resolved_voice,
            model_id: None,
            output_format: None,
            voice_settings: None,
        };
        synthesize_reply(config, text, &opts).await
    }
}

// ---------------------------------------------------------------------------
// Local Piper TTS
// ---------------------------------------------------------------------------

/// Local Piper TTS — wraps [`synthesize_piper`].
pub struct PiperTtsProvider {
    voice: String,
}

impl PiperTtsProvider {
    pub fn new(voice: impl Into<String>) -> Self {
        Self {
            voice: voice.into(),
        }
    }
}

#[async_trait]
impl TtsProvider for PiperTtsProvider {
    fn name(&self) -> &'static str {
        "piper"
    }

    async fn synthesize(
        &self,
        config: &Config,
        text: &str,
        voice: Option<&str>,
    ) -> Result<RpcOutcome<ReplySpeechResult>, String> {
        let resolved_voice = voice
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.voice.clone());
        debug!(
            "{LOG_PREFIX} piper TTS dispatch voice={} chars={}",
            resolved_voice,
            text.len()
        );
        let opts = PiperOptions {
            voice: Some(resolved_voice),
        };
        synthesize_piper(config, text, &opts).await
    }
}

// ---------------------------------------------------------------------------
// External STT provider (slug-keyed, third-party API)
// ---------------------------------------------------------------------------

/// Third-party STT provider dispatched via the voice provider registry.
/// Supports OpenAI-compatible and Deepgram API styles.
pub struct ExternalSttProvider {
    slug: String,
    model: String,
    endpoint: String,
    api_key: String,
    api_style: SttApiStyle,
}

impl ExternalSttProvider {
    pub fn new(
        slug: impl Into<String>,
        model: impl Into<String>,
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        api_style: SttApiStyle,
    ) -> Self {
        Self {
            slug: slug.into(),
            model: model.into(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            api_style,
        }
    }
}

#[async_trait]
impl SttProvider for ExternalSttProvider {
    fn name(&self) -> &'static str {
        "external"
    }

    async fn transcribe(
        &self,
        _config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String> {
        debug!(
            "{LOG_PREFIX} external STT dispatch slug={} model={} style={:?} bytes_b64={}",
            self.slug,
            self.model,
            self.api_style,
            audio_base64.len()
        );

        let audio_bytes = base64_decode(audio_base64)?;
        let mime = mime_type.unwrap_or("audio/wav");

        let result = match self.api_style {
            SttApiStyle::OpenaiAudio => {
                self.transcribe_openai_compat(&audio_bytes, mime, file_name, language)
                    .await?
            }
            SttApiStyle::Deepgram => {
                self.transcribe_deepgram(&audio_bytes, mime, language)
                    .await?
            }
        };

        Ok(RpcOutcome::single_log(
            SttResult {
                text: result,
                provider: self.slug.clone(),
            },
            &format!("voice-factory: external STT completed via {}", self.slug),
        ))
    }
}

impl ExternalSttProvider {
    async fn transcribe_openai_compat(
        &self,
        audio_bytes: &[u8],
        mime: &str,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<String, String> {
        let url = format!(
            "{}/audio/transcriptions",
            self.endpoint.trim_end_matches('/')
        );
        let ext = extension_for_mime(mime);
        let default_fname = format!("audio.{ext}");
        let fname = file_name.unwrap_or(&default_fname);

        let file_part = reqwest::multipart::Part::bytes(audio_bytes.to_vec())
            .file_name(fname.to_string())
            .mime_str(mime)
            .map_err(|e| format!("[voice-stt] mime error: {e}"))?;

        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", file_part);

        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("[voice-stt] external STT request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("[voice-stt] external STT error {status}: {body}"));
        }

        #[derive(Deserialize)]
        struct TranscriptionResp {
            text: String,
        }
        let parsed: TranscriptionResp = resp
            .json()
            .await
            .map_err(|e| format!("[voice-stt] failed to parse response: {e}"))?;
        Ok(parsed.text)
    }

    async fn transcribe_deepgram(
        &self,
        audio_bytes: &[u8],
        mime: &str,
        language: Option<&str>,
    ) -> Result<String, String> {
        let mut url = format!(
            "{}/listen?model={}",
            self.endpoint.trim_end_matches('/'),
            self.model
        );
        if let Some(lang) = language {
            url.push_str(&format!("&language={lang}"));
        }

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Content-Type", mime)
            .body(audio_bytes.to_vec())
            .send()
            .await
            .map_err(|e| format!("[voice-stt] deepgram request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("[voice-stt] deepgram error {status}: {body}"));
        }

        #[derive(Deserialize)]
        struct DeepgramChannel {
            alternatives: Vec<DeepgramAlt>,
        }
        #[derive(Deserialize)]
        struct DeepgramAlt {
            transcript: String,
        }
        #[derive(Deserialize)]
        struct DeepgramResult {
            channels: Vec<DeepgramChannel>,
        }
        #[derive(Deserialize)]
        struct DeepgramResp {
            results: DeepgramResult,
        }

        let parsed: DeepgramResp = resp
            .json()
            .await
            .map_err(|e| format!("[voice-stt] deepgram parse error: {e}"))?;

        let text = parsed
            .results
            .channels
            .first()
            .and_then(|ch| ch.alternatives.first())
            .map(|a| a.transcript.clone())
            .unwrap_or_default();
        Ok(text)
    }
}

// ---------------------------------------------------------------------------
// External TTS provider (slug-keyed, third-party API)
// ---------------------------------------------------------------------------

/// Third-party TTS provider dispatched via the voice provider registry.
/// Supports OpenAI-compatible and ElevenLabs API styles.
pub struct ExternalTtsProvider {
    slug: String,
    default_voice: String,
    endpoint: String,
    api_key: String,
    api_style: TtsApiStyle,
}

impl ExternalTtsProvider {
    pub fn new(
        slug: impl Into<String>,
        default_voice: impl Into<String>,
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        api_style: TtsApiStyle,
    ) -> Self {
        Self {
            slug: slug.into(),
            default_voice: default_voice.into(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            api_style,
        }
    }
}

#[async_trait]
impl TtsProvider for ExternalTtsProvider {
    fn name(&self) -> &'static str {
        "external"
    }

    async fn synthesize(
        &self,
        _config: &Config,
        text: &str,
        voice: Option<&str>,
    ) -> Result<RpcOutcome<ReplySpeechResult>, String> {
        let resolved_voice = voice
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&self.default_voice);

        debug!(
            "{LOG_PREFIX} external TTS dispatch slug={} voice={} style={:?} chars={}",
            self.slug,
            resolved_voice,
            self.api_style,
            text.len()
        );

        let (audio_bytes, audio_mime) = match self.api_style {
            TtsApiStyle::OpenaiAudio => self.synthesize_openai_compat(text, resolved_voice).await?,
            TtsApiStyle::ElevenLabs => self.synthesize_elevenlabs(text, resolved_voice).await?,
        };

        use base64::Engine;
        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

        Ok(RpcOutcome::single_log(
            ReplySpeechResult {
                audio_base64,
                audio_mime,
                visemes: Vec::new(),
                alignment: None,
            },
            &format!("voice-factory: external TTS completed via {}", self.slug),
        ))
    }
}

impl ExternalTtsProvider {
    async fn synthesize_openai_compat(
        &self,
        text: &str,
        voice: &str,
    ) -> Result<(Vec<u8>, String), String> {
        let url = format!("{}/audio/speech", self.endpoint.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": "tts-1",
            "voice": voice,
            "input": text,
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("[voice-tts] external TTS request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("[voice-tts] external TTS error {status}: {body}"));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("audio/mpeg")
            .to_string();

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("[voice-tts] failed to read audio: {e}"))?;

        Ok((bytes.to_vec(), content_type))
    }

    async fn synthesize_elevenlabs(
        &self,
        text: &str,
        voice_id: &str,
    ) -> Result<(Vec<u8>, String), String> {
        let url = format!(
            "{}/text-to-speech/{}",
            self.endpoint.trim_end_matches('/'),
            voice_id
        );

        let body = serde_json::json!({
            "text": text,
            "model_id": "eleven_multilingual_v2",
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("[voice-tts] elevenlabs request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("[voice-tts] elevenlabs error {status}: {body}"));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("audio/mpeg")
            .to_string();

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("[voice-tts] failed to read elevenlabs audio: {e}"))?;

        Ok((bytes.to_vec(), content_type))
    }
}

// ---------------------------------------------------------------------------
// Slug:model helpers
// ---------------------------------------------------------------------------

/// Split a provider string into `(slug, model)`.
///
/// `"deepgram:nova-2"` → `("deepgram", "nova-2")`
/// `"deepgram"` → `("deepgram", "")`
fn split_slug_model(s: &str) -> (&str, &str) {
    match s.find(':') {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => (s, ""),
    }
}

/// Resolve the effective STT provider string from config.
///
/// Precedence: `config.stt_provider` → `config.local_ai.stt_provider` → `"cloud"`.
pub fn effective_stt_provider(config: &Config) -> String {
    config
        .stt_provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let legacy = config.local_ai.stt_provider.as_str();
            if legacy.trim().is_empty() {
                None
            } else {
                Some(legacy)
            }
        })
        .unwrap_or("cloud")
        .to_string()
}

/// Resolve the effective TTS provider string from config.
///
/// Precedence: `config.tts_provider` → `config.local_ai.tts_provider` → `"cloud"`.
pub fn effective_tts_provider(config: &Config) -> String {
    config
        .tts_provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let legacy = config.local_ai.tts_provider.as_str();
            if legacy.trim().is_empty() {
                None
            } else {
                Some(legacy)
            }
        })
        .unwrap_or("cloud")
        .to_string()
}

/// Create an STT provider by looking up a slug in `config.voice_providers`.
fn create_stt_provider_by_slug(
    slug: &str,
    model: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn SttProvider>> {
    let entry = config
        .voice_providers
        .iter()
        .find(|p| p.slug == slug)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no voice provider with slug '{}' found in voice_providers",
                slug
            )
        })?;

    if !entry.capability.supports_stt() {
        return Err(anyhow::anyhow!(
            "voice provider '{}' does not support STT (capability: {})",
            slug,
            entry.capability.as_str()
        ));
    }

    let effective_model = if model.trim().is_empty() {
        entry.default_stt_model.as_deref().unwrap_or("default")
    } else {
        model
    };

    let api_key = crate::openhuman::inference::provider::factory::lookup_key_for_slug(slug, config)
        .unwrap_or_default();

    debug!(
        "{LOG_PREFIX} creating external STT provider slug={slug} model={effective_model} \
         endpoint={} key_present={}",
        entry.endpoint,
        !api_key.is_empty()
    );

    Ok(Box::new(ExternalSttProvider::new(
        slug,
        effective_model,
        &entry.endpoint,
        api_key,
        entry.stt_api_style,
    )))
}

/// Create a TTS provider by looking up a slug in `config.voice_providers`.
fn create_tts_provider_by_slug(
    slug: &str,
    voice: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn TtsProvider>> {
    let entry = config
        .voice_providers
        .iter()
        .find(|p| p.slug == slug)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no voice provider with slug '{}' found in voice_providers",
                slug
            )
        })?;

    if !entry.capability.supports_tts() {
        return Err(anyhow::anyhow!(
            "voice provider '{}' does not support TTS (capability: {})",
            slug,
            entry.capability.as_str()
        ));
    }

    let effective_voice = if voice.trim().is_empty() {
        entry.default_tts_voice.as_deref().unwrap_or("default")
    } else {
        voice
    };

    let api_key = crate::openhuman::inference::provider::factory::lookup_key_for_slug(slug, config)
        .unwrap_or_default();

    debug!(
        "{LOG_PREFIX} creating external TTS provider slug={slug} voice={effective_voice} \
         endpoint={} key_present={}",
        entry.endpoint,
        !api_key.is_empty()
    );

    Ok(Box::new(ExternalTtsProvider::new(
        slug,
        effective_voice,
        &entry.endpoint,
        api_key,
        entry.tts_api_style,
    )))
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("[voice-factory] base64 decode error: {e}"))
}

fn extension_for_mime(mime: &str) -> &str {
    match mime {
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" => "ogg",
        "audio/webm" => "webm",
        "audio/flac" => "flac",
        "audio/mp4" | "audio/m4a" => "m4a",
        _ => "wav",
    }
}

// ---------------------------------------------------------------------------
// Factory entry points (mirrors embeddings/factory.rs)
// ---------------------------------------------------------------------------

/// Creates a speech-to-text provider based on the specified name and model.
///
/// Supported provider names:
/// - `"cloud"` → backend Whisper proxy — default, preferred for laptops
///   without local models
/// - `"whisper"` → local whisper.cpp via `WHISPER_BIN` (or in-process
///   `whisper-rs` when configured)
///
/// Returns an error for unrecognised provider names so configuration
/// mistakes surface immediately rather than silently degrading.
///
/// The factory does not eagerly resolve the binary — `WhisperSttProvider`
/// looks up `WHISPER_BIN` lazily inside `transcribe()` so a misconfigured
/// install fails at use-time with a clear error message instead of at
/// startup.
pub fn create_stt_provider(
    provider: &str,
    model: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn SttProvider>> {
    debug!("{LOG_PREFIX} create_stt_provider provider={provider} model={model}");
    let model = if model.trim().is_empty() {
        DEFAULT_WHISPER_MODEL
    } else {
        model
    };
    match provider.trim() {
        "cloud" | "openhuman" => Ok(Box::new(CloudSttProvider::new(
            super::cloud_transcribe_default_model(),
        ))),
        "whisper" => Ok(Box::new(WhisperSttProvider::new(model))),
        other => {
            let (slug, slug_model) = split_slug_model(other);
            let effective_model = if slug_model.is_empty() {
                model
            } else {
                slug_model
            };
            create_stt_provider_by_slug(slug, effective_model, config)
        }
    }
}

/// Creates a text-to-speech provider based on the specified name and voice.
///
/// Supported provider names:
/// - `"cloud"` → backend ElevenLabs proxy with viseme alignment
/// - `"piper"` → local Piper subprocess via `PIPER_BIN`
///
/// Kokoro is **not** implemented in this cut — the integration shipped with
/// Piper because `PIPER_BIN` is already reserved in `.env.example` and the
/// runtime contract (subprocess + `.onnx` model) is simpler. Adding Kokoro
/// later is straightforward: add a new branch here and a `local_speech_kokoro`
/// sibling module.
pub fn create_tts_provider(
    provider: &str,
    voice: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn TtsProvider>> {
    debug!("{LOG_PREFIX} create_tts_provider provider={provider} voice={voice}");
    let voice = if voice.trim().is_empty() {
        DEFAULT_PIPER_VOICE
    } else {
        voice
    };
    match provider.trim() {
        "cloud" | "openhuman" => Ok(Box::new(CloudTtsProvider::new(if voice.is_empty() {
            None
        } else {
            Some(voice.to_string())
        }))),
        "piper" => Ok(Box::new(PiperTtsProvider::new(voice))),
        other => {
            let (slug, slug_voice) = split_slug_model(other);
            let effective_voice = if slug_voice.is_empty() {
                voice
            } else {
                slug_voice
            };
            create_tts_provider_by_slug(slug, effective_voice, config)
        }
    }
}

/// Default Whisper model. `whisper-large-v3-turbo` is the recommended ship
/// default — best accuracy-to-latency tradeoff in the Whisper family (5×
/// faster than `large-v3` with comparable WER on English). Users on lower-
/// spec hardware can drop down to `medium` / `small` / `base` / `tiny` via
/// the install presets.
pub const DEFAULT_WHISPER_MODEL: &str = "whisper-large-v3-turbo";

/// Default Piper voice — `en_US-lessac-medium`, matches
/// [`super::super::local_ai::model_ids::effective_tts_voice_id`].
pub const DEFAULT_PIPER_VOICE: &str = "en_US-lessac-medium";

/// Whisper install presets (size tiers exposed to the installer UI).
/// Mirrors the Ollama model installer surface: each entry is `(id, label)`.
pub const WHISPER_MODEL_PRESETS: &[(&str, &str)] = &[
    ("tiny", "Tiny (39 MB, fastest)"),
    ("base", "Base (74 MB)"),
    ("small", "Small (244 MB)"),
    ("medium", "Medium (769 MB, recommended)"),
    ("large-v3-turbo", "Large v3 Turbo (1.5 GB, best accuracy)"),
];

/// Returns a thread-safe default STT provider (cloud). Used by callers that
/// can't easily plumb a `Config` reference but still need a sensible default.
pub fn default_stt_provider() -> Arc<dyn SttProvider> {
    Arc::new(CloudSttProvider::new(
        super::cloud_transcribe_default_model(),
    ))
}

/// Returns a thread-safe default TTS provider (cloud).
pub fn default_tts_provider() -> Arc<dyn TtsProvider> {
    Arc::new(CloudTtsProvider::new(None))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn stt_factory_cloud_branch() {
        let p = create_stt_provider("cloud", "ignored", &cfg()).unwrap();
        assert_eq!(p.name(), "cloud");
    }

    #[test]
    fn stt_factory_whisper_branch() {
        let p = create_stt_provider("whisper", "whisper-large-v3-turbo", &cfg()).unwrap();
        assert_eq!(p.name(), "whisper");
    }

    #[test]
    fn stt_factory_whisper_empty_model_uses_default() {
        // Empty model → default whisper-large-v3-turbo; constructor must not
        // reject an empty string with an opaque error.
        let p = create_stt_provider("whisper", "", &cfg()).unwrap();
        assert_eq!(p.name(), "whisper");
    }

    #[test]
    fn stt_factory_openhuman_sentinel() {
        let p = create_stt_provider("openhuman", "ignored", &cfg()).unwrap();
        assert_eq!(p.name(), "cloud");
    }

    #[test]
    fn stt_factory_slug_without_registry_errors() {
        let err = create_stt_provider("deepgram", "nova-2", &cfg())
            .err()
            .expect("deepgram without registry entry must error");
        let msg = err.to_string();
        assert!(msg.contains("deepgram"), "should name the slug: {msg}");
        assert!(
            msg.contains("no voice provider"),
            "should explain missing: {msg}"
        );
    }

    #[test]
    fn stt_factory_slug_colon_model_resolves_with_registry() {
        let mut config = cfg();
        config.voice_providers.push(
            crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
                slug: "deepgram".into(),
                endpoint: "https://api.deepgram.com/v1".into(),
                capability: VoiceCapability::Stt,
                stt_api_style: SttApiStyle::Deepgram,
                ..Default::default()
            },
        );
        let p = create_stt_provider("deepgram:nova-2", "", &config).unwrap();
        assert_eq!(p.name(), "external");
    }

    #[test]
    fn stt_factory_bare_slug_resolves_with_registry() {
        let mut config = cfg();
        config.voice_providers.push(
            crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
                slug: "openai".into(),
                endpoint: "https://api.openai.com/v1".into(),
                capability: VoiceCapability::Both,
                default_stt_model: Some("whisper-1".into()),
                ..Default::default()
            },
        );
        let p = create_stt_provider("openai", "", &config).unwrap();
        assert_eq!(p.name(), "external");
    }

    #[test]
    fn stt_factory_tts_only_provider_rejects() {
        let mut config = cfg();
        config.voice_providers.push(
            crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
                slug: "elevenlabs".into(),
                endpoint: "https://api.elevenlabs.io/v1".into(),
                capability: VoiceCapability::Tts,
                ..Default::default()
            },
        );
        let err = create_stt_provider("elevenlabs", "model", &config)
            .err()
            .expect("TTS-only provider must reject STT");
        assert!(err.to_string().contains("does not support STT"));
    }

    #[test]
    fn stt_factory_empty_string_errors() {
        let err = create_stt_provider("", "model", &cfg())
            .err()
            .expect("empty provider must error");
        assert!(err.to_string().contains("no voice provider"));
    }

    #[test]
    fn tts_factory_cloud_branch() {
        let p = create_tts_provider("cloud", "Rachel", &cfg()).unwrap();
        assert_eq!(p.name(), "cloud");
    }

    #[test]
    fn tts_factory_piper_branch() {
        let p = create_tts_provider("piper", "en_US-lessac-medium", &cfg()).unwrap();
        assert_eq!(p.name(), "piper");
    }

    #[test]
    fn tts_factory_piper_empty_voice_uses_default() {
        let p = create_tts_provider("piper", "", &cfg()).unwrap();
        assert_eq!(p.name(), "piper");
    }

    #[test]
    fn tts_factory_openhuman_sentinel() {
        let p = create_tts_provider("openhuman", "alloy", &cfg()).unwrap();
        assert_eq!(p.name(), "cloud");
    }

    #[test]
    fn tts_factory_slug_without_registry_errors() {
        let err = create_tts_provider("kokoro", "af_bella", &cfg())
            .err()
            .expect("kokoro without registry entry must error");
        let msg = err.to_string();
        assert!(msg.contains("kokoro"), "should name the slug: {msg}");
        assert!(
            msg.contains("no voice provider"),
            "should explain missing: {msg}"
        );
    }

    #[test]
    fn tts_factory_slug_colon_voice_resolves_with_registry() {
        let mut config = cfg();
        config.voice_providers.push(
            crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
                slug: "openai".into(),
                endpoint: "https://api.openai.com/v1".into(),
                capability: VoiceCapability::Both,
                default_tts_voice: Some("alloy".into()),
                ..Default::default()
            },
        );
        let p = create_tts_provider("openai:shimmer", "", &config).unwrap();
        assert_eq!(p.name(), "external");
    }

    #[test]
    fn tts_factory_stt_only_provider_rejects() {
        let mut config = cfg();
        config.voice_providers.push(
            crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
                slug: "deepgram".into(),
                endpoint: "https://api.deepgram.com/v1".into(),
                capability: VoiceCapability::Stt,
                ..Default::default()
            },
        );
        let err = create_tts_provider("deepgram", "voice", &config)
            .err()
            .expect("STT-only provider must reject TTS");
        assert!(err.to_string().contains("does not support TTS"));
    }

    #[test]
    fn whisper_presets_cover_full_size_ladder() {
        // Sanity-check the installer surface: tiny→large-v3-turbo must all be
        // exposed so the local-AI panel can render the size picker without
        // hard-coding the list.
        let ids: Vec<&str> = WHISPER_MODEL_PRESETS.iter().map(|(id, _)| *id).collect();
        for expected in ["tiny", "base", "small", "medium", "large-v3-turbo"] {
            assert!(
                ids.contains(&expected),
                "WHISPER_MODEL_PRESETS missing {expected}"
            );
        }
    }

    #[tokio::test]
    async fn whisper_provider_fails_clearly_when_binary_missing() {
        // No WHISPER_BIN env, no model file — the provider must surface an
        // actionable error rather than panic. Drive a small base64 payload
        // so we never reach the actual transcription call.
        let _guard = unset_env_guard("WHISPER_BIN");
        let provider = WhisperSttProvider::new("whisper-large-v3-turbo");
        let result = provider
            .transcribe(&cfg(), "AAAA", Some("audio/wav"), None, None)
            .await;
        assert!(result.is_err(), "missing binary must error");
        let msg = result.err().unwrap();
        // Whatever the underlying message says, it must NOT be a serialize
        // panic — i.e. we must have hit the binary-resolution branch.
        assert!(
            !msg.is_empty(),
            "error message should be populated for diagnosis"
        );
    }

    #[test]
    fn default_providers_return_cloud() {
        assert_eq!(default_stt_provider().name(), "cloud");
        assert_eq!(default_tts_provider().name(), "cloud");
    }

    // ── slug:model parsing ──────────────────────────────────────────────

    #[test]
    fn split_slug_model_with_colon() {
        assert_eq!(split_slug_model("deepgram:nova-2"), ("deepgram", "nova-2"));
    }

    #[test]
    fn split_slug_model_bare_slug() {
        assert_eq!(split_slug_model("deepgram"), ("deepgram", ""));
    }

    #[test]
    fn split_slug_model_multiple_colons() {
        assert_eq!(split_slug_model("custom:model:v2"), ("custom", "model:v2"));
    }

    // ── effective provider resolution ───────────────────────────────────

    #[test]
    fn effective_stt_prefers_new_field() {
        let mut config = cfg();
        config.stt_provider = Some("deepgram:nova-2".into());
        config.local_ai.stt_provider = "whisper".into();
        assert_eq!(effective_stt_provider(&config), "deepgram:nova-2");
    }

    #[test]
    fn effective_stt_falls_back_to_legacy() {
        let mut config = cfg();
        config.stt_provider = None;
        config.local_ai.stt_provider = "whisper".into();
        assert_eq!(effective_stt_provider(&config), "whisper");
    }

    #[test]
    fn effective_stt_defaults_to_cloud() {
        let mut config = cfg();
        config.stt_provider = None;
        config.local_ai.stt_provider = String::new();
        assert_eq!(effective_stt_provider(&config), "cloud");
    }

    #[test]
    fn effective_tts_prefers_new_field() {
        let mut config = cfg();
        config.tts_provider = Some("openai:alloy".into());
        config.local_ai.tts_provider = "piper".into();
        assert_eq!(effective_tts_provider(&config), "openai:alloy");
    }

    #[test]
    fn effective_tts_falls_back_to_legacy() {
        let mut config = cfg();
        config.tts_provider = None;
        config.local_ai.tts_provider = "piper".into();
        assert_eq!(effective_tts_provider(&config), "piper");
    }

    #[test]
    fn effective_tts_defaults_to_cloud() {
        let config = cfg();
        assert_eq!(effective_tts_provider(&config), "cloud");
    }

    /// Drop guard that unsets an env var on construction and restores it on
    /// drop. Necessary because cargo runs tests in parallel and bare
    /// `remove_var` would leak across tests.
    fn unset_env_guard(key: &'static str) -> EnvUnsetGuard {
        let prev = std::env::var_os(key);
        std::env::remove_var(key);
        EnvUnsetGuard { key, prev }
    }

    struct EnvUnsetGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }
    impl Drop for EnvUnsetGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
