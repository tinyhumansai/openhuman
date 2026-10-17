//! TTS provider implementations: cloud, local Piper, and external (slug-keyed).

use async_trait::async_trait;
use log::debug;

use super::super::local_speech::{synthesize_piper, PiperOptions};
use super::super::reply_speech::{synthesize_reply, ReplySpeechOptions};
use super::traits::TtsProvider;
use crate::config::Config;
use crate::core::Outcome;
use tinyinference_voice::external_tts::{ExternalTtsClient, TtsApiStyle};
use tinyinference_voice::reply::ReplySpeech as ReplySpeechResult;

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
    ) -> Result<Outcome<ReplySpeechResult>, String> {
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

    #[cfg(test)]
    fn configured_voice(&self) -> Option<&str> {
        self.voice.as_deref()
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
    endpoint: String,
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
        let endpoint: String = endpoint.into();
        Self {
            slug: slug.into(),
            endpoint: endpoint.clone(),
            default_voice: default_voice.into(),
            api_style,
            client: ExternalTtsClient::new(
                super::super::egress::speech_http_client(),
                endpoint,
                api_key,
                api_style,
            ),
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

        super::super::egress::disclose_speech_transfer(
            &self.slug,
            &self.endpoint,
            crate::security::egress::DataKind::Prompt,
        )?;
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
