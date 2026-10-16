//! TTS provider implementations: local Piper and external (slug-keyed). The
//! hosted provider is registered by the hosted-backend layer.

use async_trait::async_trait;
use log::debug;

use super::super::local_speech::{synthesize_piper, PiperOptions};
use super::traits::TtsProvider;
use crate::config::Config;
use crate::core::Outcome;
use tinyinference_voice::external_tts::{ExternalTtsClient, TtsApiStyle};
use tinyinference_voice::reply::ReplySpeech as ReplySpeechResult;

const LOG_PREFIX: &str = "[voice-factory]";

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
    ) -> Result<Outcome<ReplySpeechResult>, String> {
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

    #[cfg(test)]
    fn configured_voice(&self) -> Option<&str> {
        Some(&self.voice)
    }
}

// ---------------------------------------------------------------------------
// External TTS provider (slug-keyed, third-party API)
// ---------------------------------------------------------------------------

/// Third-party TTS provider dispatched via the voice provider registry.
/// Supports OpenAI-compatible and ElevenLabs API styles; the HTTP clients
/// live in `tinyinference_voice::external_tts`.
pub struct ExternalTtsProvider {
    slug: String,
    default_voice: String,
    api_style: TtsApiStyle,
    client: ExternalTtsClient,
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
            api_style,
            client: ExternalTtsClient::new(reqwest::Client::new(), endpoint, api_key, api_style),
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
    ) -> Result<Outcome<ReplySpeechResult>, String> {
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

        let (audio_bytes, audio_mime) = self.client.synthesize(text, resolved_voice).await?;

        use base64::Engine;
        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

        Ok(Outcome::single_log(
            ReplySpeechResult {
                audio_base64,
                audio_mime,
                visemes: Vec::new(),
                alignment: None,
            },
            format!("voice-factory: external TTS completed via {}", self.slug),
        ))
    }

    #[cfg(test)]
    fn configured_voice(&self) -> Option<&str> {
        Some(&self.default_voice)
    }
}
