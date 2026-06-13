//! TTS provider implementations: cloud, local Piper, and external (slug-keyed).

use async_trait::async_trait;
use log::debug;

use super::super::local_speech::{synthesize_piper, PiperOptions};
use super::super::reply_speech::{synthesize_reply, ReplySpeechOptions, ReplySpeechResult};
use super::traits::TtsProvider;
use crate::openhuman::config::schema::voice_providers::TtsApiStyle;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice-factory]";

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
