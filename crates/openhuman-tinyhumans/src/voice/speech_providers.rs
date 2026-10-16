//! The hosted STT and TTS providers, registered under the historical slugs.
//!
//! [`CloudSttProvider`] wraps [`transcribe_cloud`] and [`CloudTtsProvider`]
//! wraps [`synthesize_reply`]. Both are registered with the core's voice
//! registry (see [`register`]) under the slugs configs already carry
//! (`cloud`, `openhuman`, and for STT also `backend`), so the factory resolves
//! them by configured slug exactly as it resolves any third-party provider.

use async_trait::async_trait;
use log::debug;
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::Outcome;
use openhuman_embed::__host::voice_registry::{
    register_stt_provider, register_tts_provider, SttProvider, SttResult, TtsProvider,
};
use tinyinference_voice::cloud::{CloudTranscribeOptions, CloudTranscribeResult};
use tinyinference_voice::reply::ReplySpeech;

use super::cloud_transcribe::transcribe_cloud;
use super::reply_speech::{synthesize_reply, ReplySpeechOptions};

const LOG_PREFIX: &str = "[voice-factory]";

/// STT slugs the hosted provider answers to.
pub const STT_SLUGS: [&str; 3] = ["cloud", "openhuman", "backend"];
/// TTS slugs the hosted provider answers to.
pub const TTS_SLUGS: [&str; 2] = ["cloud", "openhuman"];
/// Default STT model id for the hosted transcription proxy, used when the
/// caller names no model.
pub const DEFAULT_CLOUD_STT_MODEL: &str = "whisper-v1";

/// Cloud STT. Stateless; cheap to construct.
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
    ) -> Result<Outcome<SttResult>, String> {
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
        Ok(Outcome::single_log(
            SttResult {
                text,
                provider: "cloud".to_string(),
            },
            "voice-factory: cloud STT completed",
        ))
    }
}

/// Cloud TTS. `voice` is the constructor's voice; `None` lets the backend pick.
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
    ) -> Result<Outcome<ReplySpeech>, String> {
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

/// The model the hosted STT provider is built with: the caller's, else the
/// proxy default.
fn stt_model(requested: &str) -> &str {
    let requested = requested.trim();
    if requested.is_empty() {
        DEFAULT_CLOUD_STT_MODEL
    } else {
        requested
    }
}

/// Registers the hosted STT and TTS providers under their slugs.
pub(super) fn register() {
    for slug in STT_SLUGS {
        register_stt_provider(slug, |model, _config| {
            Ok(Box::new(CloudSttProvider::new(stt_model(model))))
        });
    }
    for slug in TTS_SLUGS {
        register_tts_provider(slug, |voice, _config| {
            Ok(Box::new(CloudTtsProvider::new(voice.map(str::to_string))))
        });
    }
    log::debug!("{LOG_PREFIX} registered hosted STT/TTS providers");
}

#[cfg(test)]
#[path = "speech_providers_tests.rs"]
mod tests;
